/// Shared helpers for the ignored live tests in this directory.
///
/// Every integration test binary that needs the same
/// environment-variable parsing declares `mod common;` and
/// re-exports these helpers, so seed lists, overlay ids, and
/// the soft-skip assertion behave identically across live test
/// binaries.
use std::net::SocketAddr;

use tonutils_overlay::{PeerId, SeedPeer};

/// Reads a 32-byte hex value from an environment variable.
///
/// An unset or empty variable yields `None`; a malformed value
/// panics, because a live test cannot run with a wrong
/// configuration.
pub fn configured_bytes(variable: &str) -> Option<[u8; 32]> {
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

/// Resolves the seed set shared by the mempool live tests: either the
/// `;`-separated `TON_MEMPOOL_LIVE_SEEDS` list, or a single
/// `TON_MEMPOOL_LIVE_SEED` address paired with `TON_MEMPOOL_LIVE_PEER_KEY`.
pub fn configured_live_seeds() -> Vec<SeedPeer> {
    match std::env::var("TON_MEMPOOL_LIVE_SEEDS") {
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
    }
}

/// Parses a `;`-separated list of `KEY@IP:PORT` seed entries.
pub fn parse_live_seeds(value: &str) -> Vec<SeedPeer> {
    value
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
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

/// Overlay id used by the discovery live tests, defaulting to the mainnet
/// mempool overlay that CI also uses for the delivery test.
pub fn discovery_overlay_id() -> [u8; 32] {
    configured_bytes("TON_MEMPOOL_LIVE_OVERLAY_ID").unwrap_or_else(|| {
        let hex = "12b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649";
        hex::decode(hex)
            .expect("static overlay id must be hex")
            .try_into()
            .expect("static overlay id must contain 32 bytes")
    })
}

/// Shared assertion for the discovery live tests: a discovery run either
/// yields usable peers or, with `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` set,
/// logs and passes instead of failing on transport unavailability.
pub fn assert_discovery_peers(kind: &str, peers: Vec<SeedPeer>, allow_unavailable: bool) {
    eprintln!(
        "live overlay discovery kind={kind} phase=lookup peers={}",
        peers.len()
    );
    if peers.is_empty() {
        if allow_unavailable {
            log::debug!("skipping unavailable {kind} overlay discovery: no peers");
            return;
        }
        panic!("{kind} overlay discovery returned no peers");
    }
    assert!(
        peers.iter().all(SeedPeer::is_valid),
        "{kind} overlay discovery must return usable KEY@IP:PORT peers"
    );
}
