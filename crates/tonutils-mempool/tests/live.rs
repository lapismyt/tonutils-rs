use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use tonutils_adnl::AdnlUdpSession;
use tonutils_adnl::{KeyPair, PublicKey};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_mempool::{MempoolConfig, MempoolEvent};
use tonutils_network_config::extract_dht_addresses;
use tonutils_overlay::{OverlayId, PeerId, SeedPeer};
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

async fn probe_dht_from_config(variable: &str, allow_unavailable: bool) {
    let json = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    let candidates =
        extract_dht_addresses(&json).expect("global config must contain DHT static nodes");
    let local = KeyPair::generate(&mut rand::rngs::OsRng);
    let mut last_error = None;
    for candidate in candidates.into_iter().take(16) {
        let Some(public_key) = candidate.public_key else {
            continue;
        };
        let Some(remote_public) = PublicKey::from_bytes(public_key) else {
            continue;
        };
        let mut session = match tokio::time::timeout(
            Duration::from_secs(5),
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().unwrap(),
                candidate.address,
                local,
                remote_public,
            ),
        )
        .await
        {
            Err(_timeout) => {
                last_error = Some(format!("connect timeout for {}", candidate.address));
                continue;
            }
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                continue;
            }
            Ok(Ok(session)) => session,
        };
        match session
            .dht_find_node(Int256::random(), 8, Duration::from_secs(5))
            .await
        {
            Ok(nodes) if !nodes.nodes.is_empty() => return,
            Ok(_) => {
                log::debug!("DHT probe {} returned no verified nodes", candidate.address);
                last_error = Some("empty verified DHT node response".into());
            }
            Err(error) => {
                log::debug!("DHT probe {} failed: {error}", candidate.address);
                last_error = Some(error.to_string());
            }
        }
    }
    let error = last_error.unwrap_or_else(|| "no usable peers".into());
    if allow_unavailable {
        log::debug!("skipping unavailable DHT probe: {error}");
    } else {
        panic!("no configured DHT peer answered: {error}");
    }
}

#[tokio::test]
#[ignore = "requires live mainnet QUIC access"]
async fn mainnet_dht_seed_answers_find_node() {
    let allow_unavailable = std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").is_ok();
    probe_dht_from_config("TON_GLOBAL_CONFIG_JSON", allow_unavailable).await;
}

#[tokio::test]
#[ignore = "requires live testnet QUIC access"]
async fn testnet_dht_seed_answers_find_node() {
    probe_dht_from_config("TON_TESTNET_GLOBAL_CONFIG_JSON", true).await;
}

fn configured_bytes(variable: &str) -> Option<[u8; 32]> {
    let value = match std::env::var(variable) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(error) => panic!("failed to read {variable}: {error}"),
    };
    if value.trim().is_empty() {
        return None;
    }
    let bytes =
        hex::decode(value).unwrap_or_else(|error| panic!("{variable} must be hex: {error}"));
    bytes
        .try_into()
        .ok()
        .or_else(|| panic!("{variable} must contain 32 bytes"))
}

#[tokio::test]
#[ignore = "requires a configured live overlay seed and external-message traffic"]
async fn configured_seed_delivers_valid_external_message() {
    let _ = pretty_env_logger::try_init();
    let seeds = match std::env::var("TON_MEMPOOL_LIVE_SEEDS") {
        Ok(value) if !value.trim().is_empty() => parse_live_seeds(&value),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            let seed_address: SocketAddr = match std::env::var("TON_MEMPOOL_LIVE_SEED") {
                Ok(value) => value
                    .parse()
                    .expect("TON_MEMPOOL_LIVE_SEED must be IP:port"),
                Err(error) => panic!("TON_MEMPOOL_LIVE_SEED must be set to IP:port: {error}"),
            };
            let peer_key = configured_bytes("TON_MEMPOOL_LIVE_PEER_KEY")
                .expect("TON_MEMPOOL_LIVE_PEER_KEY must be set to 32-byte hex");
            vec![SeedPeer {
                peer: PeerId::from_bytes(peer_key),
                address: seed_address.to_string(),
            }]
        }
        Err(error) => panic!("failed to read TON_MEMPOOL_LIVE_SEEDS: {error}"),
    };
    let overlay_bytes = configured_bytes("TON_MEMPOOL_LIVE_OVERLAY_ID")
        .expect("TON_MEMPOOL_LIVE_OVERLAY_ID must be set to 32-byte hex");
    let timeout = std::env::var("TON_MEMPOOL_LIVE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let overlay = OverlayId::from_bytes(overlay_bytes);
    let dht_overlay_key: [u8; 32] = overlay_bytes;
    let (_scanner, manager, stream) = MempoolScannerBuilder::new()
        .download_config(false)
        .overlay_id(overlay)
        .config(MempoolConfig::default())
        .seeds(seeds)
        .reconnect_attempts(2)
        .discovery_timeout(Duration::from_secs(30))
        .dht_overlay_key(dht_overlay_key)
        .native_udp(
            "0.0.0.0:0".parse().unwrap(),
            local_key,
            Some(Duration::from_secs(10)),
        )
        .start()
        .await
        .expect("configured seed scanner must start");
    let mut events = Box::pin(stream);
    let event = tokio::time::timeout(Duration::from_secs(timeout), async {
        loop {
            if let Some(event @ MempoolEvent::ExternalMessage { .. }) = events.next().await {
                break event;
            }
        }
    })
    .await;
    let event = event.expect("configured overlay seed did not deliver an external message");
    let lazy = event
        .lazy_message()
        .expect("external event must expose lazy message");
    let message = lazy
        .decode()
        .expect("live payload must decode as a TL-B message");
    assert!(matches!(
        message.info,
        tonutils_tlb::CommonMsgInfo::ExternalIn { .. }
    ));
    manager.shutdown_wait().await;
}

fn parse_live_seeds(value: &str) -> Vec<SeedPeer> {
    value
        .split(';')
        .map(|entry| {
            let (key, address) = entry
                .split_once('@')
                .unwrap_or_else(|| panic!("live seed must use KEY@IP:PORT: {entry}"));
            let peer_key = hex::decode(key).expect("live seed key must be hex");
            let peer_key: [u8; 32] = peer_key
                .try_into()
                .expect("live seed key must contain 32 bytes");
            address
                .parse::<SocketAddr>()
                .unwrap_or_else(|_| panic!("live seed address must be IP:PORT: {address}"));
            SeedPeer::from_public_key(peer_key, address)
        })
        .collect()
}

#[test]
fn configured_seed_parser_accepts_ipv4_and_ipv6() {
    for value in ["127.0.0.1:30303", "[::1]:30303"] {
        assert!(SocketAddr::from_str(value).is_ok());
    }
}
