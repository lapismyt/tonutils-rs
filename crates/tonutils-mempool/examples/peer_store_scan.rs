//! Mempool scanner with a durable peer store.
//!
//! Same flow as `mempool_scan`, but every peer the run learns is persisted to
//! a JSON file (`mempool-peer-store.json` by default, override with the second
//! argument or `TON_MEMPOOL_PEER_STORE`), so the next launch starts warm: the
//! best peers by stored priority are dialed immediately and the blocking
//! discovery lookup is skipped.
//!
//! Usage:
//!
//! ```bash
//! cargo run --example peer_store_scan -- [global-config.json] [peer-store.json]
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::match_wildcard_for_single_variants,
    clippy::format_collect,
    clippy::ignored_unit_patterns
)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tonutils_adnl::KeyPair;
use tonutils_mempool::{MempoolEvent, MempoolScannerBuilder};

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

/// Store path: second argument, then `TON_MEMPOOL_PEER_STORE`, then the
/// working directory's default file.
fn store_path(explicit: Option<String>) -> PathBuf {
    explicit
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("TON_MEMPOOL_PEER_STORE")
                .ok()
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("mempool-peer-store.json"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pretty_env_logger = pretty_env_logger::try_init();
    let _ = pretty_env_logger;

    let mut args = std::env::args().skip(1);
    let config_path = args.next();
    let peer_store = store_path(args.next());
    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");
    let warm = std::fs::metadata(&peer_store).is_ok();

    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;

    let mut builder = MempoolScannerBuilder::new()
        .testnet(testnet)
        .download_config(config_path.is_none())
        .discovery_timeout(Duration::from_secs(30))
        .peer_store(&peer_store)
        .peer_store_save_interval(Duration::from_secs(30))
        .native_udp_for_shard_public(
            local_addr,
            local_key,
            0,
            i64::MIN,
            MAINNET_ZERO_STATE_HASH,
            None,
        );

    if let Some(path) = config_path {
        let json = std::fs::read_to_string(&path)?;
        builder = builder.global_config_json(json);
    }

    println!(
        "peer store: {} ({})",
        peer_store.display(),
        if warm {
            "warm, skipping the blocking discovery"
        } else {
            "cold, first run discovers the roster"
        }
    );
    println!("starting mempool scanner...");
    let (_scanner, manager, stream) = builder.start().await?;
    println!("connected. listening for external messages...\n");

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
                // The store writer flushes one final time on shutdown; give it
                // a moment before the process exits.
                tokio::time::sleep(Duration::from_millis(300)).await;
                break;
            }
            _ = stat_interval.tick() => {
                let (acc, dup, rej, bytes) = stats.snapshot();
                let elapsed = start.elapsed().as_secs_f64();
                let rate = acc as f64 / elapsed;
                println!(
                    "[{elapsed:.0}s] accepted={acc} dup={dup} rejected={rej} bytes={bytes} rate={rate:.1}/s"
                );
            }
            event = stream.next() => {
                let Some(event) = event else { break };
                match event {
                    MempoolEvent::ExternalMessage { hash, raw_boc, destination, .. } => {
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
                            "[msg #{acc}] {hash_hex}..{}B{dest}",
                            raw_boc.len(),
                            acc = stats.accepted.load(Ordering::Relaxed),
                            dest = dest_info,
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
    match std::fs::metadata(&peer_store) {
        Ok(meta) => println!(
            "peer store:  {} ({} bytes)",
            peer_store.display(),
            meta.len()
        ),
        Err(error) => println!(
            "peer store:  not written ({}): {error}",
            peer_store.display()
        ),
    }

    Ok(())
}
