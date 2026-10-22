//! Stream live external messages from the mainnet mempool overlay.
//!
//! The scanner joins the mainnet basechain mempool overlay over native
//! ADNL/UDP, discovers members through the DHT, publishes its own
//! address so members can push broadcasts back, and yields every
//! external message a member relays to it.  This needs outbound UDP
//! reachability to the mainnet seeds from the global config; no lite
//! servers and no API keys are involved.
//!
//! ```bash
//! cargo run -p tonutils-mempool --example mempool_stream
//! ```

use std::net::SocketAddr;
use std::time::Duration;

use futures::StreamExt;
use tonutils_adnl::KeyPair;
use tonutils_mempool::{MempoolEvent, MempoolScannerBuilder};

/// Mainnet zero state file hash, the root of the basechain overlay id.
const MAINNET_ZERO_STATE_HASH: [u8; 32] = [
    0x5e, 0x99, 0x4f, 0xcf, 0x4d, 0x42, 0x5c, 0x0a, 0x6c, 0xe6, 0xa7, 0x92, 0x59, 0x4b, 0x71, 0x73,
    0x20, 0x5f, 0x74, 0x0a, 0x39, 0xcd, 0x56, 0xf5, 0x37, 0xde, 0xfd, 0x28, 0xb4, 0x8a, 0x0f, 0x6e,
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = pretty_env_logger::try_init();

    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;
    let (scanner, manager, events) = MempoolScannerBuilder::new()
        .download_config(true)
        .discovery_timeout(Duration::from_secs(30))
        // The shard-public helper derives the basechain overlay id
        // from the zero state hash and wires the DHT discovery
        // lookup over the same overlay key, which is what turns
        // bootstrap seeds into an actual member list.
        .native_udp_for_shard_public(
            local_addr,
            KeyPair::generate(&mut rand::rngs::OsRng),
            0,
            i64::MIN,
            MAINNET_ZERO_STATE_HASH,
            None,
        )
        .start()
        .await?;

    println!("joined the mainnet mempool overlay; Ctrl-C to stop");
    let mut events = Box::pin(events);
    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { break };
                match event {
                    MempoolEvent::ExternalMessage { hash, raw_boc, routing, .. } => {
                        println!(
                            "message {}.. from peer {:?} ({} bytes)",
                            hex::encode(&hash[..8]),
                            routing.peer,
                            raw_boc.len(),
                        );
                        // Relay what arrived to the members this
                        // scanner is connected to: mempool delivery
                        // is sampling-based, so every connected
                        // holder that re-broadcasts widens the fan-out.
                        if let Err(error) = scanner.send_external(raw_boc).await {
                            eprintln!("re-broadcast failed: {error}");
                        }
                    }
                    MempoolEvent::Included { hash, .. } => {
                        println!(
                            "message {}.. reached a block",
                            hex::encode(&hash[..8]),
                        );
                    }
                    MempoolEvent::PeerStatus(status) => {
                        log::debug!("peer status: {status:?}");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    println!("shutting down");
    manager.shutdown_wait().await;
    Ok(())
}
