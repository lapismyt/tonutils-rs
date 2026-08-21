use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_network_config::extract_dht_addresses;
use tonutils_tl::Int256;

fn configured_seed(variable: &str) -> SocketAddr {
    let value = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    value
        .parse()
        .unwrap_or_else(|_| panic!("{variable} must contain an IP:port seed"))
}

#[tokio::test]
#[ignore = "requires a configured mainnet seed and live network access"]
async fn mainnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_MAINNET_SEED");
}

#[tokio::test]
#[ignore = "requires a configured testnet seed and live network access"]
async fn testnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_TESTNET_SEED");
}

#[tokio::test]
#[ignore = "requires live network access"]
async fn configured_global_config_can_start_scanner() {
    let json = std::env::var("TON_GLOBAL_CONFIG_JSON")
        .expect("set TON_GLOBAL_CONFIG_JSON to run this ignored live test");
    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");
    let result = MempoolScannerBuilder::new()
        .testnet(testnet)
        .global_config_json(json)
        .download_config(false)
        .start()
        .await;
    let (_, manager, _) = result.expect("configured global config must yield bootstrap peers");
    manager.shutdown();
}

async fn probe_dht_from_config(variable: &str) {
    let json = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    let candidates =
        extract_dht_addresses(&json).expect("global config must contain DHT static nodes");
    let local = KeyPair::generate(&mut rand::rngs::OsRng);
    let mut last_error = None;
    for candidate in candidates.into_iter().take(8) {
        let Some(public_key) = candidate.public_key else {
            continue;
        };
        let Some(remote_public) = PublicKey::from_bytes(public_key) else {
            continue;
        };
        let mut session = match AdnlUdpSession::connect(
            "0.0.0.0:0".parse().unwrap(),
            candidate.address,
            local,
            remote_public,
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        match session
            .dht_find_node(Int256::random(), 8, Duration::from_secs(2))
            .await
        {
            Ok(nodes) if !nodes.nodes.is_empty() => return,
            Ok(_) => {
                eprintln!("DHT probe {} returned no verified nodes", candidate.address);
                last_error = Some("empty verified DHT node response".into());
            }
            Err(error) => {
                eprintln!("DHT probe {} failed: {error}", candidate.address);
                last_error = Some(error.to_string());
            }
        }
    }
    let error = last_error.unwrap_or_else(|| "no usable peers".into());
    if std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").as_deref() == Ok("1") {
        eprintln!("skipping unavailable DHT probe: {error}");
    } else {
        panic!("no configured DHT peer answered: {error}");
    }
}

#[tokio::test]
#[ignore = "requires live mainnet UDP access"]
async fn mainnet_dht_seed_answers_find_node() {
    probe_dht_from_config("TON_GLOBAL_CONFIG_JSON").await;
}

#[tokio::test]
#[ignore = "requires live testnet UDP access"]
async fn testnet_dht_seed_answers_find_node() {
    probe_dht_from_config("TON_TESTNET_GLOBAL_CONFIG_JSON").await;
}

#[test]
fn configured_seed_parser_accepts_ipv4_and_ipv6() {
    for value in ["127.0.0.1:30303", "[::1]:30303"] {
        assert!(SocketAddr::from_str(value).is_ok());
    }
}
