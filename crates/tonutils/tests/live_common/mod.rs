#![allow(dead_code)]

use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use tonutils::{
    adnl::{
        AdnlAddress, AdnlPublicKey, now_i32,
        tl::{AdnlIdShort, AdnlTlPublicKey},
    },
    network_config,
    overlay::{
        OverlayId, SeedPeer,
        tl::{OverlayNode, OverlayNodeToSign},
    },
    tl::TlInt256 as Int256,
};

pub const DEFAULT_OVERLAY_ID: &str =
    "12b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649";

pub fn load_global_config(variable: &str) -> Result<String> {
    let json = env::var(variable).with_context(|| format!("set {variable}"))?;
    if json.trim().is_empty() {
        bail!("{variable} is empty");
    }
    Ok(json)
}

pub fn configured_seeds(variable: &str) -> Result<Vec<SeedPeer>> {
    let json = load_global_config(variable)?;
    let candidates = network_config::extract_dht_addresses(&json)
        .with_context(|| format!("{variable} has no DHT static nodes"))?;
    let seeds = candidates
        .into_iter()
        .filter_map(|candidate| {
            candidate
                .public_key
                .map(|key| SeedPeer::from_public_key(key, candidate.address.to_string()))
        })
        .filter(SeedPeer::is_valid)
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        bail!("{variable} produced no valid DHT seeds");
    }
    Ok(seeds)
}

pub fn configured_overlay_id() -> Result<OverlayId> {
    let value =
        env::var("TON_MEMPOOL_LIVE_OVERLAY_ID").unwrap_or_else(|_| DEFAULT_OVERLAY_ID.to_owned());
    let bytes = hex::decode(value.trim()).context("TON_MEMPOOL_LIVE_OVERLAY_ID must be hex")?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
        anyhow::anyhow!("overlay id must be 32 bytes, got {}", bytes.len())
    })?;
    Ok(OverlayId::from_bytes(bytes))
}

pub fn valid_overlay_nodes(nodes: &[OverlayNode], overlay: OverlayId) -> usize {
    nodes
        .iter()
        .filter(|node| valid_overlay_node(node, overlay))
        .count()
}

fn valid_overlay_node(node: &OverlayNode, overlay: OverlayId) -> bool {
    let now = now_i32();
    if node.overlay.0 != overlay.as_bytes() || node.version < now.saturating_sub(600) {
        return false;
    }
    let AdnlTlPublicKey::Ed25519 { key } = &node.id else {
        return false;
    };
    let Some(public_key) = AdnlPublicKey::from_bytes(key.0) else {
        return false;
    };
    let adnl_id = AdnlAddress::from(&public_key).to_bytes();
    let unsigned = OverlayNodeToSign {
        id: AdnlIdShort {
            id: Int256(adnl_id),
        },
        overlay: node.overlay.clone(),
        version: node.version,
    };
    let signature = match node.signature.as_slice() {
        signature if signature.len() == 64 => signature,
        signature if signature.len() == 68 => &signature[4..],
        _ => return false,
    };
    let Ok(signature) = signature.try_into() else {
        return false;
    };
    public_key.verify_raw(&tl_proto::serialize(unsigned), &signature)
}

pub fn parse_socket_addr(address: &str) -> Result<SocketAddr> {
    address
        .parse()
        .with_context(|| format!("invalid socket address {address}"))
}

pub fn elapsed_millis(started: std::time::Instant) -> u128 {
    started.elapsed().as_millis()
}

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
pub const DISCOVERY_OUTER_TIMEOUT: Duration = Duration::from_secs(45);
