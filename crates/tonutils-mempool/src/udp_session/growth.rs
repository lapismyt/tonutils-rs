//! Post-bootstrap peer growth for the native UDP path.
//!
//! Split out of `lookup.rs` to keep both files inside the repository's 1000
//! line limit.  Growth expands the overlay session pool after bootstrap:
//! every round asks a known member for `overlay.getRandomPeers`, validates
//! the returned `overlay.node` records, and resolves their DHT `address`
//! values so members reached only transitively become connectable.
//!
//! Address resolution mirrors pytoniq's `OverlayManager.get_more_peers`
//! plus `DhtClient.get_overlay_node`: candidates are resolved through the
//! DHT resolver seeds (the config bootstrap nodes) with a `dht.findValue`
//! that follows `dht.valueNotFound` closer nodes, never through the overlay
//! member that answered `getRandomPeers` alone - members that are pytoniq
//! clients themselves do not answer `dht.findValue` at all.

use super::*;

use super::lookup::{dht_key_id, resolve_address_on_session};

/// Builds the periodic peer-growth lookup used after bootstrap.
///
/// Bootstrap settles for whatever the DHT `nodes` value and the first
/// `overlay.getRandomPeers` round can reach, which is a handful of members.
/// Upstream drains `pending_peers_` at 60 nodes a minute, so the chance of
/// being pinged scales with the number of members this node is queued at -
/// pytoniq grows the same way, from the DHT seed set to `max_peers = 30`.
///
/// Each round asks the next known member for `overlay.getRandomPeers`,
/// validates the returned `overlay.node` records against `overlay` and
/// resolves their DHT `address` values, which is what makes members reached
/// only transitively connectable.  Re-issuing the query also re-announces this
/// node, refreshing the `version` of its signed record in that peer's queue.
///
/// Without resolver seeds, candidate addresses fall back to the answering
/// member's own session; prefer [`udp_peer_growth_with_resolvers`] when the
/// bootstrap seeds are known, since they are the nodes guaranteed to answer
/// `dht.findValue`.
pub fn udp_peer_growth(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    udp_peer_growth_with_resolvers(
        local_addr,
        local_keypair,
        overlay,
        timeout,
        Arc::new(tokio::sync::RwLock::new(Vec::new())),
    )
}

/// [`udp_peer_growth`] with a DHT resolver seed set shared with the caller.
///
/// The resolver list is the bootstrap seed set - explicit seeds and the
/// global config's `dht.static_nodes` - which are DHT nodes by construction,
/// the same node set pytoniq's `DhtClient` resolves `address` values
/// through.  `MempoolScannerBuilder::start` fills the shared list after
/// bootstrap resolution.
pub fn udp_peer_growth_with_resolvers(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    timeout: Duration,
    resolvers: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
) -> SeedDiscoveryLookup {
    let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Arc::new(move |seeds: Vec<SeedPeer>| {
        let cursor = Arc::clone(&cursor);
        let resolvers = Arc::clone(&resolvers);
        Box::pin(async move {
            if seeds.is_empty() {
                return Vec::new();
            }
            let index = cursor.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % seeds.len();
            let seed = seeds[index].clone();
            let Some(remote) = AdnlPublicKey::from_bytes(seed.peer.as_bytes()) else {
                return Vec::new();
            };
            let Ok(address) = seed.address.parse() else {
                return Vec::new();
            };
            let resolvers = resolvers.read().await.clone();
            match query_overlay_random_peers(
                local_addr,
                local_keypair,
                remote,
                address,
                overlay,
                timeout,
                &resolvers,
            )
            .await
            {
                Some(found) => {
                    log::debug!(
                        "udp_peer_growth: {} candidate member(s) from {}",
                        found.len(),
                        seed.address
                    );
                    found
                }
                None => {
                    log::debug!("udp_peer_growth: no candidates from {}", seed.address);
                    Vec::new()
                }
            }
        })
    })
}

/// Asks one member for `overlay.getRandomPeers` and resolves the returned
/// candidates' DHT `address` records into connectable `SeedPeer`s.
///
/// `getRandomPeers` runs under `timeout.min(5s)`; every candidate's address
/// is then resolved in parallel with a fresh `timeout` budget, bounded by a
/// shared deadline.  Each candidate tries the rotating DHT resolver seed
/// first and falls back to the answering member's session, so a member that
/// does not run a DHT query handler costs one bounded hop rather than the
/// whole lookup.
#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
pub(super) async fn query_overlay_random_peers(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    overlay: OverlayId,
    timeout: Duration,
    resolvers: &[SeedPeer],
) -> Option<Vec<SeedPeer>> {
    let session = match shared_session(local_addr, local_keypair, remote, address).await {
        Ok(session) => session,
        Err(error) => {
            log::debug!("query_overlay_random_peers: connect to {address} failed: {error}");
            return None;
        }
    };
    let per_query = timeout.min(Duration::from_secs(5));
    log::debug!("query_overlay_random_peers: sending overlay.getRandomPeers to {address}");
    let nodes = match session
        .overlay_get_random_peers(tonutils_tl::Int256(overlay.as_bytes()), per_query)
        .await
    {
        Ok(nodes) => nodes,
        Err(error) => {
            log::debug!(
                "query_overlay_random_peers: overlay_get_random_peers to {address} failed: {error}"
            );
            return None;
        }
    };
    let now = now_i32();
    let total = nodes.nodes.len();
    let mut address_keys = Vec::new();
    for node in nodes.nodes {
        if !valid_overlay_node(&node, overlay, now) {
            continue;
        }
        let TlPublicKey::Ed25519 { key } = node.id else {
            continue;
        };
        let Some(overlay_public) = AdnlPublicKey::from_bytes(key.0) else {
            continue;
        };
        let adnl_id = AdnlAddress::from(&overlay_public).to_bytes();
        address_keys.push(dht_key_id(adnl_id, b"address"));
    }
    if address_keys.is_empty() {
        log::debug!(
            "query_overlay_random_peers: no valid members from {address} ({total} member(s))"
        );
        return None;
    }
    // The answer above consumed up to `per_query`; resolution gets a full
    // `timeout` budget after it, mirroring pytoniq, which resolves every
    // returned node through its DhtClient under its own timeout.
    let deadline = tokio::time::Instant::now() + timeout;
    let valid = address_keys.len();
    let resolutions = join_all(
        address_keys
            .into_iter()
            .enumerate()
            .map(|(index, address_key)| {
                let resolver = resolvers.get(index % resolvers.len().max(1)).cloned();
                async move {
                    // pytoniq parity: resolve through a DHT resolver seed first.
                    // The member that answered getRandomPeers may be a client
                    // that never answers `dht.findValue`, while the config seeds
                    // are DHT nodes; its own session stays as the fallback.
                    if let Some(resolver) = resolver
                        && let Ok(resolver_address) =
                            resolver.address.parse::<std::net::SocketAddr>()
                        && let Some(resolver_remote) =
                            AdnlPublicKey::from_bytes(resolver.peer.as_bytes())
                        && let Ok(hop) = shared_session(
                            local_addr,
                            local_keypair,
                            resolver_remote,
                            resolver_address,
                        )
                        .await
                        && let Some(found) = resolve_address_on_session(
                            local_addr,
                            &local_keypair,
                            resolver_address,
                            &hop,
                            address_key.clone(),
                            per_query,
                            deadline,
                        )
                        .await
                    {
                        return Some(found);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return None;
                    }
                    let hop = shared_session(local_addr, local_keypair, remote, address)
                        .await
                        .ok()?;
                    resolve_address_on_session(
                        local_addr,
                        &local_keypair,
                        address,
                        &hop,
                        address_key,
                        per_query,
                        deadline,
                    )
                    .await
                }
            }),
    )
    .await;

    let mut result: Vec<SeedPeer> = Vec::new();
    let mut resolved = 0usize;
    for found in resolutions.into_iter().flatten() {
        resolved += 1;
        if result
            .iter()
            .all(|candidate: &SeedPeer| candidate.peer != found.peer)
        {
            result.push(found);
        }
    }
    let unresolved = valid.saturating_sub(resolved);
    log::debug!(
        "query_overlay_random_peers: found {} peers from {address} \
         ({total} member(s): {valid} valid, {resolved} address record(s), {unresolved} unresolved)",
        result.len()
    );
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
