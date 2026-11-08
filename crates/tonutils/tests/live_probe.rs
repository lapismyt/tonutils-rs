#![cfg(all(feature = "adnl", feature = "network-config"))]

//! Mandatory live probes for the DHT request-response path.

mod live_common;

use std::time::Instant;

use anyhow::{Context, Result, bail};
use live_common::{CONNECT_TIMEOUT, QUERY_TIMEOUT, elapsed_millis, load_global_config};
use tonutils::{
    adnl::{AdnlKeyPair, AdnlPublicKey, AdnlUdpSession, now_i32, tl::DhtNodesBoxed},
    network_config,
    tl::TlInt256 as Int256,
};

const MAX_DHT_SEEDS: usize = 16;

#[tokio::test]
#[ignore = "requires live mainnet network access"]
async fn mainnet_dht_find_node_returns_valid_nodes() -> Result<()> {
    probe_dht("mainnet", "TON_GLOBAL_CONFIG_JSON").await
}

#[tokio::test]
#[ignore = "requires live testnet network access"]
async fn testnet_dht_find_node_returns_valid_nodes() -> Result<()> {
    probe_dht("testnet", "TON_TESTNET_GLOBAL_CONFIG_JSON").await
}

async fn probe_dht(network: &str, variable: &str) -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let json = load_global_config(variable)?;
    let candidates = network_config::extract_dht_addresses(&json)
        .with_context(|| format!("{variable} has no DHT static nodes"))?;
    let local_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let mut errors = Vec::new();
    for candidate in candidates.into_iter().take(MAX_DHT_SEEDS) {
        let Some(public_key) = candidate.public_key else {
            errors.push(format!("{}: missing public key", candidate.address));
            continue;
        };
        let Some(remote_key) = AdnlPublicKey::from_bytes(public_key) else {
            errors.push(format!("{}: invalid public key", candidate.address));
            continue;
        };
        let started = Instant::now();
        let session = tokio::time::timeout(
            CONNECT_TIMEOUT,
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().expect("wildcard address must parse"),
                candidate.address,
                local_key,
                remote_key,
            ),
        )
        .await;
        let session = match session {
            Err(_) => {
                errors.push(format!("{}: connect timeout", candidate.address));
                continue;
            }
            Ok(Err(error)) => {
                errors.push(format!("{}: connect error {error}", candidate.address));
                continue;
            }
            Ok(Ok(session)) => session,
        };
        let answer = tokio::time::timeout(
            QUERY_TIMEOUT,
            session.dht_find_node(Int256::random(), 8, CONNECT_TIMEOUT),
        )
        .await;
        match answer {
            Err(_) => errors.push(format!("{}: findNode timeout", candidate.address)),
            Ok(Err(error)) => {
                errors.push(format!("{}: findNode error {error}", candidate.address));
            }
            Ok(Ok(nodes)) => {
                let valid = valid_dht_nodes(&nodes);
                eprintln!(
                    "live probe kind=dht network={network} request=dht.findNode target={} peer={} elapsed_ms={} nodes={} valid={}",
                    candidate.address,
                    hex::encode(public_key),
                    elapsed_millis(started),
                    nodes.nodes.len(),
                    valid,
                );
                if !nodes.nodes.is_empty() && valid == nodes.nodes.len() {
                    return Ok(());
                }
                errors.push(format!(
                    "{}: answer nodes={} valid={valid}",
                    candidate.address,
                    nodes.nodes.len()
                ));
            }
        }
    }
    bail!(
        "no {network} DHT seed returned valid nonempty dht.findNode: {}",
        errors.join("; ")
    )
}

fn valid_dht_nodes(nodes: &DhtNodesBoxed) -> usize {
    let now = now_i32();
    nodes.nodes.iter().filter(|node| node.is_valid(now)).count()
}
