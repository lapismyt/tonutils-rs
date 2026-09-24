#![allow(clippy::large_types_passed_by_value)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tonutils_adnl::{AdnlUdpSession, KeyPair};
use tonutils_network_config::extract_dht_addresses;
use tonutils_tl::tl::network::DhtKey;

use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();

    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let local_addr: SocketAddr = "0.0.0.0:0".parse()?;
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "global.config.json".into());
    let json = std::fs::read_to_string(&config_path)?;

    let addresses = extract_dht_addresses(&json)?;

    println!(
        "Local pubkey: {}",
        hex::encode(local_key.public_key.as_bytes())
    );
    println!("Testing against {} DHT seeds...\n", addresses.len());

    // DHT key for "nodes" on the mainnet basechain overlay.
    // The hash MUST use the boxed serialization (with constructor prefix)
    // to match upstream TON's DhtKey::compute_key_id().
    let overlay_id = tonutils_tl::Int256([
        0x12, 0xb8, 0xa8, 0x3f, 0x09, 0x8e, 0x15, 0xea, 0x47, 0xfe, 0x76, 0xd0, 0xb0, 0xdf, 0x09,
        0x86, 0xff, 0x6d, 0xda, 0x19, 0x80, 0x79, 0x6b, 0x08, 0x4b, 0x0d, 0x2a, 0x68, 0xb2, 0x55,
        0x86, 0x49,
    ]);
    let dht_key = DhtKey {
        id: overlay_id.clone(),
        name: b"nodes".to_vec(),
        idx: 0,
    };
    let boxed = dht_key.boxed_bytes();
    let hash = Sha256::digest(&boxed);
    let key_hash = tonutils_tl::Int256(hash.into());
    println!("DHT key hash (boxed): {}", key_hash.to_hex());
    println!(
        "Boxed bytes ({} bytes): {}",
        boxed.len(),
        hex::encode(&boxed)
    );

    for (i, addr_info) in addresses.iter().enumerate() {
        let addr = addr_info.address;
        let key_bytes: [u8; 32] = addr_info.public_key.unwrap_or([0; 32]);
        let remote_pub =
            tonutils_adnl::PublicKey::from_bytes(key_bytes).ok_or("invalid public key")?;
        let adnl_id = tonutils_adnl::AdnlAddress::from(&remote_pub);

        println!("[{i}] {addr} adnl_id={}", hex::encode(adnl_id.as_bytes()));

        let start = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(10),
            test_dht_value(local_addr, local_key, remote_pub, addr, key_hash.clone()),
        )
        .await
        {
            Ok(Ok(result)) => {
                println!(
                    "    OK in {:.1}s: {}",
                    start.elapsed().as_secs_f64(),
                    result
                );
            }
            Ok(Err(e)) => {
                println!("    ERROR in {:.1}s: {e}", start.elapsed().as_secs_f64());
            }
            Err(_) => {
                println!("    TIMEOUT after {:.1}s", start.elapsed().as_secs_f64());
            }
        }
    }

    Ok(())
}

async fn test_dht_value(
    local_addr: SocketAddr,
    local_key: KeyPair,
    remote_pub: tonutils_adnl::PublicKey,
    remote_addr: SocketAddr,
    key_hash: tonutils_tl::Int256,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut session =
        AdnlUdpSession::connect(local_addr, remote_addr, local_key, remote_pub).await?;

    match session
        .dht_find_value(key_hash, 4, Duration::from_secs(8))
        .await
    {
        Ok(result) => Ok(format!("{result:?}")),
        Err(e) => Err(format!("{e}").into()),
    }
}
