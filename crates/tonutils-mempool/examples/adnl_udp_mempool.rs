#![allow(
    clippy::cast_precision_loss,
    clippy::match_wildcard_for_single_variants,
    clippy::format_collect,
    clippy::wildcard_enum_match_arm,
    clippy::ignored_unit_patterns
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::StreamExt;
use tonutils_adnl::KeyPair;
use tonutils_mempool::{MempoolEvent, MempoolScannerBuilder};

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();

    let config_path = std::env::args().nth(1);
    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");

    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;

    let network_label = if testnet { "testnet" } else { "mainnet" };
    println!("adnl-udp mempool scanner ({network_label})");
    println!("  local addr:   {local_addr}");
    println!(
        "  local pubkey: {}",
        hex::encode(local_key.public_key.as_bytes())
    );

    let start = Instant::now();

    let mut builder = MempoolScannerBuilder::new()
        .testnet(testnet)
        .download_config(config_path.is_none())
        .discovery_timeout(Duration::from_secs(30))
        .native_udp_for_shard_public(
            local_addr,
            local_key,
            0,
            i64::MIN,
            MAINNET_ZERO_STATE_HASH,
            Some(Duration::from_secs(15)),
        );

    if let Some(path) = config_path {
        let json = std::fs::read_to_string(&path)?;
        builder = builder.global_config_json(json);
    }

    println!(
        "[{:>6.1}s] starting scanner...",
        start.elapsed().as_secs_f64()
    );
    let (_scanner, manager, stream) = builder.start().await?;
    println!(
        "[{:>6.1}s] connected. listening for external messages...\n",
        start.elapsed().as_secs_f64()
    );

    let stats = Arc::new(Stats::new());
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

    Ok(())
}
