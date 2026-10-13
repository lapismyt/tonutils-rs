#![allow(
    clippy::cast_precision_loss,
    clippy::match_wildcard_for_single_variants,
    clippy::format_collect,
    clippy::ignored_unit_patterns
)]

//! Maximum-throughput receive scanner: big queues, many dedup shards,
//! quality-ranked fan-out, background peer churn, and one log line per
//! accepted external message plus a periodic peer-quality table.
//!
//! ```text
//! cargo run --example mempool_scan_fast -- [global-config.json]
//! RUST_LOG=debug cargo run --example mempool_scan_fast
//! TON_MEMPOOL_PINNED="<peer_id_hex>@<host:port>,..." cargo run --example mempool_scan_fast
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tonutils_adnl::KeyPair;
use tonutils_mempool::{MempoolConfig, MempoolEvent, MempoolScannerBuilder, PeerQualityConfig};
use tonutils_overlay::{PeerId, SeedPeer};

/// Mainnet zero state file hash.
const MAINNET_ZERO_STATE_HASH: [u8; 32] = [
    0x5e, 0x99, 0x4f, 0xcf, 0x4d, 0x42, 0x5c, 0x0a, 0x6c, 0xe6, 0xa7, 0x92, 0x59, 0x4b, 0x71, 0x73,
    0x20, 0x5f, 0x74, 0x0a, 0x39, 0xcd, 0x56, 0xf5, 0x37, 0xde, 0xfd, 0x28, 0xb4, 0x8a, 0x0f, 0x6e,
];

struct Stats {
    accepted: AtomicU64,
    duplicates: AtomicU64,
    rejected: AtomicU64,
    total_bytes: AtomicU64,
}

impl Stats {
    fn new() -> Self {
        Self {
            accepted: AtomicU64::new(0),
            duplicates: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.accepted.load(Ordering::Relaxed),
            self.duplicates.load(Ordering::Relaxed),
            self.rejected.load(Ordering::Relaxed),
            self.total_bytes.load(Ordering::Relaxed),
        )
    }
}

/// Parses `TON_MEMPOOL_PINNED` entries of the form
/// `<32-byte peer id hex>@<host:port>[,<hex>@<host:port>...]`.
///
/// Pinned peers bootstrap with reconnect, never count against
/// `overlay_max_peers`, and survive quality churn - the right place for
/// configured seeds or validators.
fn parse_pinned(spec: &str) -> Result<Vec<SeedPeer>, Box<dyn std::error::Error>> {
    spec.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (peer, address) = entry
                .split_once('@')
                .ok_or("pinned entry must be <peer_id_hex>@<host:port>")?;
            let bytes = hex::decode(peer.trim())?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "pinned peer id must be exactly 32 bytes")?;
            Ok(SeedPeer {
                peer: PeerId::from_bytes(bytes),
                address: address.trim().to_owned(),
            })
        })
        .collect()
}

fn short_peer(peer: PeerId) -> String {
    hex::encode(&peer.as_bytes()[..4])
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::try_init().ok();

    let config_path = std::env::args().nth(1);
    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");
    let pinned_spec = std::env::var("TON_MEMPOOL_PINNED").ok();

    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;

    let mut builder = MempoolScannerBuilder::new()
        .testnet(testnet)
        .download_config(config_path.is_none())
        .config(MempoolConfig {
            event_queue_capacity: 8192,
            dedup_shards: 256,
            max_dedup_entries: 500_000,
            ..MempoolConfig::default()
        })
        .discovery_timeout(Duration::from_secs(10))
        // 100 managed sessions, churned by delivery quality below.
        .overlay_max_peers(100)
        .quality_eviction(true)
        .peer_quality(PeerQualityConfig {
            late_threshold: Duration::from_millis(500),
            miss_window: Duration::from_secs(3),
            min_observations: 10,
            sweep_interval: Duration::from_secs(1),
            churn_interval: Duration::from_secs(60),
            score_halflife: Duration::from_secs(60),
            ..PeerQualityConfig::default()
        })
        .native_udp_for_shard_public(
            local_addr,
            local_key,
            0,
            i64::MIN,
            MAINNET_ZERO_STATE_HASH,
            None,
        );

    if let Some(spec) = pinned_spec {
        builder = builder.pinned_peers(parse_pinned(&spec)?);
    }
    if let Some(path) = config_path {
        builder = builder.global_config_json(std::fs::read_to_string(&path)?);
    }

    println!("starting fast mempool scanner...");
    let (scanner, manager, stream) = builder.start().await?;
    println!(
        "connected ({} session(s)). listening for external messages...\n",
        manager.peer_count().await
    );

    let stats = Arc::new(Stats::new());
    let start = Instant::now();
    let mut stream = Box::pin(stream);

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown_clone.notify_one();
    });

    let mut stat_interval = tokio::time::interval(Duration::from_secs(5));
    stat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                println!("\nshutting down...");
                manager.shutdown_wait().await;
                break;
            }
            _ = stat_interval.tick() => {
                let (acc, dup, rej, bytes) = stats.snapshot();
                let elapsed = start.elapsed().as_secs_f64();
                let rate = acc as f64 / elapsed;
                println!(
                    "[{elapsed:.0}s] accepted={acc} dup={dup} rejected={rej} bytes={bytes} rate={rate:.1}/s peers={}",
                    manager.peer_count().await,
                );
                let snapshot = scanner.quality().snapshot();
                if !snapshot.is_empty() {
                    println!("  peer quality (score / first / ontime / late / miss):");
                    for entry in snapshot.iter().take(10) {
                        println!(
                            "    {} {:+.1} {} {} {} {}",
                            short_peer(entry.peer),
                            entry.score,
                            entry.first_deliveries,
                            entry.on_time_deliveries,
                            entry.late_deliveries,
                            entry.misses,
                        );
                    }
                }
            }
            event = stream.next() => {
                let Some(event) = event else { break };
                match event {
                    MempoolEvent::ExternalMessage { hash, raw_boc, destination, routing, .. } => {
                        stats.accepted.fetch_add(1, Ordering::Relaxed);
                        stats.total_bytes.fetch_add(raw_boc.len() as u64, Ordering::Relaxed);

                        let hash_hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
                        let dest_info = match destination {
                            Some(addr) => {
                                let prefix: String = addr.iter().take(4).map(|b| format!("{b:02x}")).collect();
                                format!(" -> {prefix}..")
                            }
                            None => String::new(),
                        };

                        println!(
                            "[msg #{}] {hash_hex}..{}B from {}{dest_info}",
                            stats.accepted.load(Ordering::Relaxed),
                            raw_boc.len(),
                            short_peer(routing.peer),
                        );
                    }
                    MempoolEvent::PeerStatus(status) => {
                        log::debug!("peer status: {status:?}");
                    }
                    _ => {}
                }
            }
        }
    }

    let (acc, dup, rej, bytes) = stats.snapshot();
    let elapsed = start.elapsed().as_secs_f64();
    println!("\n--- final stats ---");
    println!("accepted:    {acc}");
    println!("duplicates:  {dup}");
    println!("rejected:    {rej}");
    println!("total bytes: {bytes}");
    println!("elapsed:     {elapsed:.1}s");
    println!("avg rate:    {:.1} msg/s", acc as f64 / elapsed);

    Ok(())
}
