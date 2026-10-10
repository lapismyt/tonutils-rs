//! Overlay, DHT, and seed discovery helpers used by the native UDP path.
//!
//! Split out of `udp_session.rs` to keep that file near the repository's
//! 1000 line limit; `udp_session` re-exports everything below.

use super::*;

use tonutils_tl::tl::network::DhtValue;

/// Returns the shared-transport session for one lookup hop.
///
/// Lookups resolve their socket through
/// [`AdnlUdpTransport::for_node`], the same transport the live
/// overlay sessions use, so a one-shot query sends from the
/// node's single source address instead of a second port the
/// peer would keep in its connection table only until the query
/// returns.  The session is per peer id: a peer that already
/// holds a live session is queried on it, and a peer that does
/// not get a fresh one that ends with the query.
#[allow(clippy::large_types_passed_by_value)]
async fn shared_session(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    remote_addr: std::net::SocketAddr,
) -> Result<AdnlUdpSession, String> {
    let transport = AdnlUdpTransport::for_node(local_addr, local_keypair)
        .await
        .map_err(|error| error.to_string())?;
    Ok(transport.session_for_peer(remote, remote_addr))
}

/// Stop resolving DHT `address` records for one seed once this many overlay
/// candidates resolved.  Seed discovery queries every configured seed in
/// parallel, so a handful of peers per seed already saturates the peer pool.
const ADDRESS_LOOKUP_TARGET: usize = 4;

/// Give up on a seed's `address` records after this many consecutive misses,
/// so a lagging peer cannot consume the whole discovery deadline.
const ADDRESS_LOOKUP_MISSES: usize = 3;

/// DHT nodes queried in parallel after one `dht.valueNotFound`.
///
/// The node that returned the overlay `nodes` record is usually not
/// responsible for a candidate's `address` key, and `dht.valueNotFound`
/// answers with the nodes closest to that key instead, so asking them - rather
/// than walking the chain one node at a time - is what keeps resolution cheap.
const ADDRESS_LOOKUP_BRANCH: usize = 3;

/// Follow-up rounds after `dht.valueNotFound` for a single address record.
const ADDRESS_LOOKUP_ROUNDS: usize = 2;

/// Extra margin above one in-flight query, so every seed hands its results
/// back before the caller's own deadline for the whole lookup.
const SLACK: Duration = Duration::from_secs(2);

/// A frontier node queried during one round of [`query_overlay_seed`], together
/// with the session that carried the query so the answer lookups can reuse it.
struct SeedAnswer {
    seed: SeedPeer,
    session: AdnlUdpSession,
    response: DhtValueResult,
}

/// Sends `dht.findValue` over an existing session.
///
/// Reusing the socket matters: upstream keeps a single `AdnlPeerPair` per
/// (local id, remote id) with one source address, so a burst of fresh sockets
/// from the same ADNL node id makes the peer flip that address between them
/// and answer only the port it saw last, leaving every other session with a
/// full timeout.
async fn query_dht_value_on_session(
    session: &mut AdnlUdpSession,
    address: std::net::SocketAddr,
    key: tonutils_tl::Int256,
    count: usize,
    timeout: Duration,
) -> Option<DhtValueResult> {
    match session
        .dht_find_value(key, count.min(i32::MAX as usize) as i32, timeout)
        .await
    {
        Ok(result) => Some(result),
        Err(error) => {
            log::debug!("query_dht_value_on_session: dht_find_value to {address} failed: {error}");
            None
        }
    }
}

/// Parses a DHT `address` record into the peer it belongs to plus its UDP
/// `ip:port`.
fn parse_address_record(value: DhtValue, source: std::net::SocketAddr) -> Option<SeedPeer> {
    let Ok(address_list) = tl_proto::deserialize::<AddressListBoxed>(&value.value) else {
        log::debug!("resolve_address: record from {source} is not an address list");
        return None;
    };
    let TlPublicKey::Ed25519 { key } = &value.key.id else {
        return None;
    };
    let peer = PeerId::from_bytes(key.0);
    for address in address_list.addrs {
        let Address::Udp { ip, port } = address else {
            continue;
        };
        let Ok(port) = u16::try_from(port) else {
            continue;
        };
        if port == 0 || ip == 0 {
            continue;
        }
        return Some(SeedPeer {
            peer,
            address: format!("{}:{port}", std::net::Ipv4Addr::from(ip.cast_unsigned())),
        });
    }
    log::debug!("resolve_address: record from {source} has no usable UDP address");
    None
}

/// Resolves one overlay candidate's DHT `address` record and returns its
/// `ip:port`.
///
/// The first hop reuses the session that just answered the overlay `nodes`
/// lookup, so no second socket to the same peer is opened.  Every follow-up
/// hop asks the nodes the DHT pointed at, because the seed that returned the
/// overlay `nodes` record is usually not responsible for the candidate's
/// `address` key and answers `dht.valueNotFound`.
async fn resolve_address_on_session(
    local_addr: std::net::SocketAddr,
    local_keypair: &KeyPair,
    seed_address: std::net::SocketAddr,
    session: &mut AdnlUdpSession,
    address_key: tonutils_tl::Int256,
    timeout: Duration,
    deadline: tokio::time::Instant,
) -> Option<SeedPeer> {
    let address_key_hex = address_key.to_hex();
    let first =
        query_dht_value_on_session(session, seed_address, address_key.clone(), 1, timeout).await;
    let mut pending = match first {
        Some(DhtValueResult::Found { value }) => return parse_address_record(value, seed_address),
        Some(DhtValueResult::NotFound { nodes }) => nodes.nodes,
        None => {
            log::debug!(
                "resolve_address: {address_key_hex} via {seed_address} got no answer (valueNotFound would have returned closer nodes)"
            );
            return None;
        }
    };
    let mut tried = std::collections::HashSet::new();
    tried.insert(seed_address.to_string());

    for round in 1..=ADDRESS_LOOKUP_ROUNDS {
        if tokio::time::Instant::now() >= deadline {
            log::debug!("resolve_address: {address_key_hex} hit its deadline");
            return None;
        }
        let take = pending.len().min(ADDRESS_LOOKUP_BRANCH);
        let leftovers = pending.split_off(take);
        let batch =
            tonutils_overlay::select_typed_dht_peers(pending, ADDRESS_LOOKUP_BRANCH, now_i32())
                .into_iter()
                .filter(|peer| tried.insert(peer.address.clone()))
                .collect::<Vec<_>>();
        pending = leftovers;
        if batch.is_empty() {
            log::debug!("resolve_address: {address_key_hex} has no untried DHT nodes left");
            return None;
        }
        let results = join_all(batch.into_iter().map(|peer| {
            let address_key = address_key.clone();
            async move {
                let Ok(remote_addr) = peer.address.parse::<std::net::SocketAddr>() else {
                    return None;
                };
                let remote = AdnlPublicKey::from_bytes(peer.peer.as_bytes())?;
                let mut hop = match shared_session(local_addr, *local_keypair, remote, remote_addr)
                    .await
                {
                    Ok(hop) => hop,
                    Err(error) => {
                        log::debug!("resolve_address: connect to {remote_addr} failed: {error}");
                        return None;
                    }
                };
                let response =
                    query_dht_value_on_session(&mut hop, remote_addr, address_key, 1, timeout)
                        .await;
                Some((remote_addr, response))
            }
        }))
        .await;

        let mut next = Vec::new();
        let mut answered = 0usize;
        let mut silent = 0usize;
        for (remote_addr, response) in results.into_iter().flatten() {
            match response {
                Some(DhtValueResult::Found { value }) => {
                    log::debug!(
                        "resolve_address: {address_key_hex} resolved at {remote_addr} on round {round}"
                    );
                    return parse_address_record(value, remote_addr);
                }
                Some(DhtValueResult::NotFound { nodes }) => {
                    answered += 1;
                    next.extend(nodes.nodes);
                }
                None => silent += 1,
            }
        }
        if next.is_empty() {
            log::debug!(
                "resolve_address: {address_key_hex} got no closer nodes on round {round} ({answered} valueNotFound, {silent} without answer)"
            );
            return None;
        }
        pending.extend(next);
    }
    log::debug!("resolve_address: {address_key_hex} gave up after {ADDRESS_LOOKUP_ROUNDS} rounds");
    None
}

pub fn udp_dht_lookup(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    node_count: i32,
    timeout: Duration,
) -> TypedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                Some(async move {
                    let session = shared_session(local_addr, local_keypair, remote, address)
                        .await
                        .ok()?;
                    session
                        .dht_find_node(tonutils_tl::Int256::random(), node_count, timeout)
                        .await
                        .ok()
                        .map(|nodes| nodes.nodes)
                })
            }))
            .await;
            responses.into_iter().flatten().flatten().collect()
        })
    })
}

pub fn udp_iterative_dht_lookup(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    node_count: i32,
    rounds: usize,
    timeout: Duration,
) -> TypedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let mut frontier = seeds;
            let mut discovered = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let now = now_i32();
            for _ in 0..rounds.max(1) {
                let responses = join_all(frontier.into_iter().filter_map(|seed| {
                    let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                    let address = seed.address.parse().ok()?;
                    Some(query_dht_seed(
                        local_addr,
                        local_keypair,
                        remote,
                        address,
                        node_count,
                        timeout,
                    ))
                }))
                .await;
                frontier = Vec::new();
                for nodes in responses.into_iter().flatten() {
                    for node in nodes {
                        let key = match &node.id {
                            tonutils_tl::tl::network::PublicKey::Ed25519 { key } => key.0,
                            _ => continue,
                        };
                        if seen.insert(key) {
                            frontier.extend(tonutils_overlay::select_typed_dht_peers(
                                [node.clone()],
                                8,
                                now,
                            ));
                            discovered.push(node);
                        }
                    }
                }
                if frontier.is_empty() {
                    break;
                }
            }
            discovered
        })
    })
}

pub fn udp_overlay_lookup(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                Some(query_overlay_seed(
                    local_addr,
                    local_keypair,
                    remote,
                    address,
                    overlay,
                    overlay_key,
                    max_records,
                    timeout,
                ))
            }))
            .await;
            let mut result = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for peers in responses.into_iter().flatten() {
                for peer in peers {
                    if seen.insert((peer.peer, peer.address.clone())) {
                        log::debug!(
                            "udp_overlay_lookup: discovered peer {} at {}",
                            hex::encode(peer.peer.as_bytes()),
                            peer.address
                        );
                        result.push(peer);
                        if result.len() >= max_records {
                            return result;
                        }
                    }
                }
            }
            log::debug!(
                "udp_overlay_lookup: returning {} peers from discovery",
                result.len()
            );
            result
        })
    })
}

#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
async fn query_overlay_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> Option<Vec<SeedPeer>> {
    let initial = SeedPeer {
        peer: PeerId::from_bytes(remote.to_bytes()),
        address: address.to_string(),
    };
    let overlay_dht_key = dht_key_id(overlay_key, b"nodes");
    let per_query_timeout = timeout.min(Duration::from_secs(5));
    log::debug!(
        "query_overlay_seed: seed={address} overlay_dht_key={}",
        overlay_dht_key.to_hex()
    );
    let mut frontier = vec![initial.clone()];
    let mut seen = std::collections::HashSet::new();
    let now = now_i32();
    // Callers such as `MempoolScannerBuilder::start` wrap this whole lookup in
    // their own deadline, and `udp_overlay_lookup` waits for every seed, so one
    // seed running to the full budget discards the results of all the others.
    // Stop one in-flight query length of time early to hand results back first.
    let deadline = tokio::time::Instant::now() + timeout.saturating_sub(per_query_timeout + SLACK);

    for _ in 0..6 {
        if tokio::time::Instant::now() >= deadline {
            log::debug!("query_overlay_seed: node lookup hit its deadline at {address}");
            break;
        }
        let responses = join_all(frontier.drain(..).filter_map(|seed| {
            let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
            let address = seed.address.parse().ok()?;
            let overlay_dht_key = overlay_dht_key.clone();
            Some(async move {
                let mut session =
                    match shared_session(local_addr, local_keypair, remote, address).await {
                        Ok(session) => session,
                        Err(error) => {
                            log::debug!("query_overlay_seed: connect to {address} failed: {error}");
                            return None;
                        }
                    };
                let response = query_dht_value_on_session(
                    &mut session,
                    address,
                    overlay_dht_key,
                    max_records,
                    per_query_timeout,
                )
                .await?;
                Some(SeedAnswer {
                    seed,
                    session,
                    response,
                })
            })
        }))
        .await;
        let mut next = Vec::new();
        for answer in responses.into_iter().flatten() {
            let SeedAnswer {
                seed,
                mut session,
                response,
            } = answer;
            let peer_address = seed.address.parse::<std::net::SocketAddr>().ok();
            match response {
                DhtValueResult::Found { value } => {
                    let nodes: OverlayNodesBoxed = tl_proto::deserialize(&value.value).ok()?;
                    log::debug!(
                        "query_overlay_seed: found {} overlay nodes from {address}",
                        nodes.nodes.len()
                    );
                    let mut candidates = Vec::new();
                    for node in nodes.nodes {
                        if !valid_overlay_node(&node, overlay, now) {
                            log::debug!(
                                "query_overlay_seed: skipping node (overlay mismatch or expired)"
                            );
                            continue;
                        }
                        if candidates.len() >= max_records {
                            continue;
                        }
                        let TlPublicKey::Ed25519 { key } = node.id else {
                            continue;
                        };
                        let Some(overlay_public) = AdnlPublicKey::from_bytes(key.0) else {
                            continue;
                        };
                        candidates.push(overlay_public);
                    }
                    // The `address` record of every candidate is another DHT
                    // round trip, so they are resolved over the very session
                    // that just answered instead of one fresh socket per
                    // candidate; see `query_dht_value_on_session`.
                    let mut result = Vec::new();
                    let mut misses = 0usize;
                    let Some(peer_address) = peer_address else {
                        log::debug!("query_overlay_seed: unusable seed address {}", seed.address);
                        continue;
                    };
                    for overlay_public in candidates {
                        if tokio::time::Instant::now() >= deadline {
                            log::debug!("query_overlay_seed: address resolution hit its deadline");
                            break;
                        }
                        let address_key =
                            dht_key_id(AdnlAddress::from(&overlay_public).to_bytes(), b"address");
                        let Some(resolved) = resolve_address_on_session(
                            local_addr,
                            &local_keypair,
                            peer_address,
                            &mut session,
                            address_key,
                            per_query_timeout,
                            deadline,
                        )
                        .await
                        else {
                            misses += 1;
                            if misses >= ADDRESS_LOOKUP_MISSES {
                                log::debug!(
                                    "query_overlay_seed: giving up on {} after {} address misses",
                                    seed.address,
                                    misses
                                );
                                break;
                            }
                            continue;
                        };
                        misses = 0;
                        if result
                            .iter()
                            .all(|candidate: &SeedPeer| candidate.peer != resolved.peer)
                        {
                            result.push(resolved);
                        }
                        if result.len() >= ADDRESS_LOOKUP_TARGET {
                            break;
                        }
                    }
                    if !result.is_empty() {
                        return Some(result);
                    }
                }
                DhtValueResult::NotFound { nodes } => {
                    log::debug!(
                        "query_overlay_seed: not found, got {} closer nodes from {address}",
                        nodes.nodes.len()
                    );
                    for seed in
                        tonutils_overlay::select_typed_dht_peers(nodes.nodes, max_records, now)
                    {
                        if seen.insert((seed.peer, seed.address.clone())) {
                            next.push(seed);
                        }
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            log::debug!("query_overlay_seed: frontier exhausted at {address}");
            break;
        }
    }

    if tokio::time::Instant::now() >= deadline {
        log::debug!("query_overlay_seed: skipping getRandomPeers fallback at {address}");
        return None;
    }
    log::debug!(
        "query_overlay_seed: DHT lookup missed, falling back to overlay getRandomPeers at {address}"
    );
    if let Some(fallback) = query_overlay_random_peers(
        local_addr,
        local_keypair,
        remote,
        address,
        overlay,
        per_query_timeout,
    )
    .await
    {
        log::debug!(
            "query_overlay_seed: overlay getRandomPeers fallback found {} peers at {address}",
            fallback.len()
        );
        return Some(fallback);
    }

    log::debug!("query_overlay_seed: returning None for {address}");
    None
}

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
pub fn udp_peer_growth(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Arc::new(move |seeds: Vec<SeedPeer>| {
        let cursor = Arc::clone(&cursor);
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
            match query_overlay_random_peers(
                local_addr,
                local_keypair,
                remote,
                address,
                overlay,
                timeout,
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

#[allow(clippy::large_types_passed_by_value)]
async fn query_overlay_random_peers(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    overlay: OverlayId,
    timeout: Duration,
) -> Option<Vec<SeedPeer>> {
    let session = match shared_session(local_addr, local_keypair, remote, address).await {
        Ok(session) => session,
        Err(error) => {
            log::debug!("query_overlay_random_peers: connect to {address} failed: {error}");
            return None;
        }
    };
    log::debug!("query_overlay_random_peers: direct UDP ADNL session established to {address}");
    let overlay_int = tonutils_tl::Int256(overlay.as_bytes());
    log::debug!("query_overlay_random_peers: sending overlay.getRandomPeers to {address}");
    let nodes = match session.overlay_get_random_peers(overlay_int, timeout).await {
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
    let mut records = 0usize;
    let mut not_found = 0usize;
    let mut no_answer = 0usize;
    let mut result = Vec::new();
    for node in nodes.nodes {
        if !valid_overlay_node(&node, overlay, now) {
            continue;
        }
        let TlPublicKey::Ed25519 { key } = node.id else {
            continue;
        };
        let overlay_public = AdnlPublicKey::from_bytes(key.0)?;
        let adnl_id = AdnlAddress::from(&overlay_public).to_bytes();
        let address_key = dht_key_id(adnl_id, b"address");
        let response = query_dht_value_seed(
            local_addr,
            local_keypair,
            remote,
            address,
            address_key,
            1,
            timeout,
        )
        .await;
        let value = match response {
            Some(DhtValueResult::Found { value }) => {
                records += 1;
                value
            }
            Some(DhtValueResult::NotFound { .. }) => {
                not_found += 1;
                continue;
            }
            None => {
                no_answer += 1;
                continue;
            }
        };
        let Ok(address_list) = tl_proto::deserialize::<AddressListBoxed>(&value.value) else {
            continue;
        };
        let Some((peer, resolved_address)) = address_list.addrs.into_iter().find_map(|address| {
            let Address::Udp { ip, port } = address else {
                return None;
            };
            let port = u16::try_from(port).ok()?;
            if port == 0 || ip == 0 {
                return None;
            }
            let TlPublicKey::Ed25519 { key } = &value.key.id else {
                return None;
            };
            Some((
                PeerId::from_bytes(key.0),
                format!("{}:{port}", std::net::Ipv4Addr::from(ip.cast_unsigned())),
            ))
        }) else {
            continue;
        };
        if result
            .iter()
            .all(|candidate: &SeedPeer| candidate.peer != peer)
        {
            result.push(SeedPeer {
                peer,
                address: resolved_address,
            });
        }
    }
    log::debug!(
        "query_overlay_random_peers: found {} peers from {address} ({total} member(s): {records} address record(s), {not_found} valueNotFound, {no_answer} without answer)",
        result.len()
    );
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

#[allow(clippy::large_types_passed_by_value)]
async fn query_dht_value_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    key: tonutils_tl::Int256,
    count: usize,
    timeout: Duration,
) -> Option<DhtValueResult> {
    let session = match shared_session(local_addr, local_keypair, remote, address).await {
        Ok(session) => session,
        Err(error) => {
            log::debug!("query_dht_value_seed: connect to {address} failed: {error}");
            return None;
        }
    };
    match session
        .dht_find_value(key, count.min(i32::MAX as usize) as i32, timeout)
        .await
    {
        Ok(result) => Some(result),
        Err(error) => {
            log::debug!("query_dht_value_seed: dht_find_value to {address} failed: {error}");
            None
        }
    }
}

fn dht_key_id(id: [u8; 32], name: &[u8]) -> tonutils_tl::Int256 {
    let dht_key = DhtKey {
        id: tonutils_tl::Int256(id),
        name: name.to_vec(),
        idx: 0,
    };
    // Hash the BOXED form (with constructor prefix) per upstream TON.
    tonutils_tl::Int256(Sha256::digest(dht_key.boxed_bytes()).into())
}

#[allow(clippy::large_types_passed_by_value)]
async fn query_dht_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    node_count: i32,
    timeout: Duration,
) -> Option<Vec<tonutils_tl::tl::network::DhtNode>> {
    let session = shared_session(local_addr, local_keypair, remote, address)
        .await
        .ok()?;
    session
        .dht_find_node(tonutils_tl::Int256::random(), node_count, timeout)
        .await
        .ok()
        .map(|nodes| nodes.nodes)
}

pub fn direct_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            Ok(Box::new(
                AdnlUdpOverlaySession::connect(
                    seed.peer,
                    local_addr,
                    seed.address
                        .parse()
                        .map_err(|error| format!("invalid seed address: {error}"))?,
                    local_keypair,
                    remote,
                )
                .await?,
            ) as Box<dyn OverlaySession>)
        })
    })
}

pub fn channel_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    timeout: Duration,
) -> crate::OverlaySessionFactory {
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            Ok(Box::new(
                AdnlUdpOverlaySession::connect_with_channel(
                    seed.peer,
                    local_addr,
                    seed.address
                        .parse()
                        .map_err(|error| format!("invalid seed address: {error}"))?,
                    local_keypair,
                    remote,
                    timeout,
                )
                .await?,
            ) as Box<dyn OverlaySession>)
        })
    })
}

pub fn overlay_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: KeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
) -> crate::OverlaySessionFactory {
    // One cache for every session this factory spawns: a member admitted
    // through one peer becomes gossip for all of them, which is what keeps
    // answers to `overlay.getRandomPeers` from collapsing to this node alone.
    let shared = OverlayMemberCache::default();
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        let shared = shared.clone();
        Box::pin(async move {
            let remote = remote.ok_or_else(|| "seed peer is not a valid Ed25519 key".to_owned())?;
            let mut session = match channel_timeout {
                Some(timeout) => {
                    AdnlUdpOverlaySession::connect_for_overlay_with_channel(
                        seed.peer,
                        overlay,
                        local_addr,
                        seed.address
                            .parse()
                            .map_err(|error| format!("invalid seed address: {error}"))?,
                        local_keypair,
                        remote,
                        timeout,
                    )
                    .await?
                }
                None => {
                    AdnlUdpOverlaySession::connect_for_overlay(
                        seed.peer,
                        overlay,
                        local_addr,
                        seed.address
                            .parse()
                            .map_err(|error| format!("invalid seed address: {error}"))?,
                        local_keypair,
                        remote,
                    )
                    .await?
                }
            };
            session.members = shared;
            Ok(Box::new(session) as Box<dyn OverlaySession>)
        })
    })
}
