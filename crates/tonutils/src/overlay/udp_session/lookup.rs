#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "ADNL and overlay wire fields and bounded counters intentionally use protocol widths"
)]
//! Overlay, DHT, and seed discovery helpers used by the native UDP path.
//!
//! Split out of `udp_session.rs` to keep that file near the repository's
//! 1000 line limit; `udp_session` re-exports everything below.

use std::{sync::Arc, time::Duration};

use crate::{
    adnl::{
        AdnlAddress, AdnlKeyPair, AdnlPublicKey, AdnlUdpSession, AdnlUdpTransport, now_i32,
        tl::{
            AddressListBoxed, AdnlNetAddress as Address, AdnlTlPublicKey as TlPublicKey, DhtKey,
            DhtNode, DhtValue, DhtValueResult,
        },
    },
    overlay::{
        AdnlUdpOverlaySession, OverlayMemberEndpoints, OverlaySessionFactory,
        inbound::OverlayMemberCache,
        packet_dedup::PacketPreDedup,
        plumtree::PlumtreeHandle,
        record_member, record_member_quic,
        udp_session::{growth::query_overlay_random_peers, valid_overlay_node},
    },
    overlay::{
        OverlayId, OverlaySession, PeerId, SeedDiscoveryLookup, SeedPeer, TypedDiscoveryLookup,
        select_typed_dht_peers, tl::OverlayNodesBoxed,
    },
    tl::TlInt256 as Int256,
};

use futures::{StreamExt, future::join_all};
use sha2::{Digest, Sha256};

/// Returns the shared-transport session for one lookup hop.
///
/// Lookups resolve their socket through
/// [`AdnlUdpTransport::for_node`], the same transport the live
/// overlay sessions use, so a one-shot query sends from the
/// node's single source address instead of a second port the
/// peer would keep in its connection table only until the query
/// returns.  The session is per peer id: a peer that already
/// holds a live session is queried on it at the address the
/// manager established, and a peer that does not get a fresh one
/// bound to `remote_addr` that ends with the query.
///
/// The lookup never adopts `remote_addr` on an established
/// session.  The endpoints one member advertises are candidates,
/// and a sweep over them used to repoint the session's single
/// send address at whichever candidate adopted last - the
/// manager's keepalives included.  Probes that dial one specific
/// endpoint carry it on the query instead
/// (`AdnlUdpSession::overlay_get_random_peers_to`).
pub(crate) async fn shared_session(
    local_addr: std::net::SocketAddr,
    local_keypair: &AdnlKeyPair,
    remote: AdnlPublicKey,
    remote_addr: std::net::SocketAddr,
) -> Result<AdnlUdpSession, String> {
    let transport = AdnlUdpTransport::for_node(local_addr, *local_keypair)
        .await
        .map_err(|error| error.to_string())?;
    Ok(transport.session_for_probe(remote, remote_addr))
}

/// How many candidate `address` records one seed resolves concurrently.
///
/// Every candidate behind the overlay-node value is resolved before the
/// caller's deadline, not just the first few: the value is the overlay's
/// full-node phonebook, and a member whose address this node never resolves
/// can never verify it.  The bound keeps one seed from opening more parallel
/// DHT dialogs than the transport can carry.
const ADDRESS_LOOKUP_CONCURRENCY: usize = 12;

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
pub(super) async fn query_dht_value_on_session(
    session: &AdnlUdpSession,
    address: std::net::SocketAddr,
    key: Int256,
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
///
/// Both endpoints land in `endpoints`: the UDP one the scanner connects on,
/// and an advertised `adnl.address.quic` entry when the record carries one,
/// which upstream `QuicSender::get_ip_address` prefers over the `+1000`
/// fallback.  `pub(crate)` so the QUIC discovery path records the same fields
/// from the records it resolves.
pub(crate) fn parse_address_record(
    value: &DhtValue,
    source: std::net::SocketAddr,
    endpoints: &OverlayMemberEndpoints,
) -> Option<SeedPeer> {
    let Ok(address_list) = tl_proto::deserialize::<AddressListBoxed>(&value.value) else {
        log::debug!("resolve_address: record from {source} is not an address list");
        return None;
    };
    let TlPublicKey::Ed25519 { key } = &value.key.id else {
        return None;
    };
    let peer = PeerId::from_bytes(key.0);
    for address in &address_list.addrs {
        let Address::Quic { ip, port } = address else {
            continue;
        };
        if *port == 0 || *ip == 0 {
            continue;
        }
        let Ok(port) = u16::try_from(*port) else {
            continue;
        };
        let quic = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(ip.cast_unsigned())),
            port,
        );
        record_member_quic(endpoints, peer, quic);
    }
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
        let udp = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(ip.cast_unsigned())),
            port,
        );
        record_member(endpoints, peer, udp);
        return Some(SeedPeer {
            peer,
            address: format!("{udp}"),
        });
    }
    log::debug!("resolve_address: record from {source} has no usable UDP address");
    None
}

/// Resolves one overlay candidate's DHT `address` record and returns its
/// `ip:port`.
///
/// The first hop reuses an existing session - the one that just answered the
/// overlay `nodes` lookup during seed discovery, or a config DHT resolver
/// seed during peer growth - so no second socket to the same peer is opened.
/// Every follow-up hop asks the nodes the DHT pointed at, because the first
/// hop is usually not responsible for the candidate's `address` key and
/// answers `dht.valueNotFound`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_address_on_session(
    local_addr: std::net::SocketAddr,
    local_keypair: &AdnlKeyPair,
    seed_address: std::net::SocketAddr,
    session: &AdnlUdpSession,
    address_key: Int256,
    timeout: Duration,
    deadline: tokio::time::Instant,
    endpoints: &OverlayMemberEndpoints,
) -> Option<SeedPeer> {
    let address_key_hex = address_key.to_hex();
    let first =
        query_dht_value_on_session(session, seed_address, address_key.clone(), 1, timeout).await;
    let mut pending = match first {
        Some(DhtValueResult::Found { value }) => {
            return parse_address_record(&value, seed_address, endpoints);
        }
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
        let batch = select_typed_dht_peers(pending, ADDRESS_LOOKUP_BRANCH, now_i32())
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
                let hop = match shared_session(local_addr, local_keypair, remote, remote_addr).await
                {
                    Ok(hop) => hop,
                    Err(error) => {
                        log::debug!("resolve_address: connect to {remote_addr} failed: {error}");
                        return None;
                    }
                };
                let response =
                    query_dht_value_on_session(&hop, remote_addr, address_key, 1, timeout).await;
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
                    return parse_address_record(&value, remote_addr, endpoints);
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
    local_keypair: AdnlKeyPair,
    node_count: i32,
    timeout: Duration,
) -> TypedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        Box::pin(async move {
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                Some(async move {
                    let session = shared_session(local_addr, &local_keypair, remote, address)
                        .await
                        .ok()?;
                    session
                        .dht_find_node(Int256::random(), node_count, timeout)
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
    local_keypair: AdnlKeyPair,
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
                            TlPublicKey::Ed25519 { key } => key.0,
                            _ => continue,
                        };
                        if seen.insert(key) {
                            frontier.extend(select_typed_dht_peers([node.clone()], 8, now));
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
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
) -> SeedDiscoveryLookup {
    udp_overlay_lookup_with_endpoints(
        local_addr,
        local_keypair,
        overlay,
        overlay_key,
        max_records,
        timeout,
        OverlayMemberEndpoints::default(),
    )
}

/// [`udp_overlay_lookup`] that also records every resolved member endpoint
/// into `endpoints`.
///
/// The registry is what lets the QUIC discovery path target overlay members
/// (full nodes, which run a QUIC server on `adnl_udp_port + 1000`) instead
/// of the bootstrap `dht.static_nodes`, whose `dht-server` binary has no
/// QUIC listener at all. See [`crate::overlay::OverlayMemberEndpoints`].
pub fn udp_overlay_lookup_with_endpoints(
    local_addr: std::net::SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
    endpoints: OverlayMemberEndpoints,
) -> SeedDiscoveryLookup {
    Arc::new(move |seeds: Vec<SeedPeer>| {
        let endpoints = Arc::clone(&endpoints);
        Box::pin(async move {
            // The bootstrap seeds are DHT nodes, so they double as the
            // resolver set for candidate address lookups - the same role
            // pytoniq's DhtClient node set plays.
            let resolvers = seeds.clone();
            // A shared reference so every per-seed future borrows the one
            // resolver list instead of moving it.
            let resolvers = &resolvers;
            let responses = join_all(seeds.into_iter().filter_map(|seed| {
                let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes())?;
                let address = seed.address.parse().ok()?;
                let endpoints = Arc::clone(&endpoints);
                Some(async move {
                    query_overlay_seed(
                        local_addr,
                        &local_keypair,
                        remote,
                        address,
                        overlay,
                        overlay_key,
                        max_records,
                        timeout,
                        resolvers,
                        &endpoints,
                    )
                    .await
                })
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

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::too_many_lines,
    reason = "The UDP seed lookup keeps DHT traversal, fallback probing, and address resolution in one auditable flow"
)]
async fn query_overlay_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: &AdnlKeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    overlay: OverlayId,
    overlay_key: [u8; 32],
    max_records: usize,
    timeout: Duration,
    resolvers: &[SeedPeer],
    endpoints: &OverlayMemberEndpoints,
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
    // Callers such as `OverlayNodeBuilder::start_client` wrap this whole lookup in
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
                let session = match shared_session(local_addr, local_keypair, remote, address).await
                {
                    Ok(session) => session,
                    Err(error) => {
                        log::debug!("query_overlay_seed: connect to {address} failed: {error}");
                        return None;
                    }
                };
                let response = query_dht_value_on_session(
                    &session,
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
                session,
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
                    // round trip.  Every candidate is resolved over the very
                    // session that just answered instead of one fresh socket
                    // per candidate (see `query_dht_value_on_session`), and the
                    // resolutions run concurrently: one seed's value can list
                    // the whole overlay member set, and skipping entries would
                    // trade a burst of DHT lookups for members this node can
                    // never verify.  The caller's deadline ends the round, not
                    // an entry cap.
                    let Some(peer_address) = peer_address else {
                        log::debug!("query_overlay_seed: unusable seed address {}", seed.address);
                        continue;
                    };
                    let candidate_count = candidates.len();
                    let lookups =
                        futures::stream::iter(candidates.into_iter().map(|overlay_public| {
                            let session = &session;
                            async move {
                                if tokio::time::Instant::now() >= deadline {
                                    return None;
                                }
                                let address_key = dht_key_id(
                                    AdnlAddress::from(&overlay_public).to_bytes(),
                                    b"address",
                                );
                                resolve_address_on_session(
                                    local_addr,
                                    local_keypair,
                                    peer_address,
                                    session,
                                    address_key,
                                    per_query_timeout,
                                    deadline,
                                    endpoints,
                                )
                                .await
                            }
                        }))
                        .buffer_unordered(ADDRESS_LOOKUP_CONCURRENCY);
                    let resolved = lookups.collect::<Vec<Option<SeedPeer>>>().await;
                    let mut result = Vec::new();
                    for resolved in resolved.into_iter().flatten() {
                        if result
                            .iter()
                            .all(|candidate: &SeedPeer| candidate.peer != resolved.peer)
                        {
                            result.push(resolved);
                        }
                        if result.len() >= max_records {
                            break;
                        }
                    }
                    if !result.is_empty() {
                        log::debug!(
                            "query_overlay_seed: resolved {} of {candidate_count} member address(es) from {address}",
                            result.len()
                        );
                        return Some(result);
                    }
                    log::debug!(
                        "query_overlay_seed: resolved none of the {candidate_count} member address(es) from {address}"
                    );
                }
                DhtValueResult::NotFound { nodes } => {
                    log::debug!(
                        "query_overlay_seed: not found, got {} closer nodes from {address}",
                        nodes.nodes.len()
                    );
                    for seed in select_typed_dht_peers(nodes.nodes, max_records, now) {
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
        resolvers,
        endpoints,
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

pub(crate) fn dht_key_id(id: [u8; 32], name: &[u8]) -> Int256 {
    let dht_key = DhtKey {
        id: Int256(id),
        name: name.to_vec(),
        idx: 0,
    };
    // Hash the BOXED form (with constructor prefix) per upstream TON.
    Int256(Sha256::digest(dht_key.boxed_bytes()).into())
}

#[expect(
    clippy::large_types_passed_by_value,
    reason = "The signature intentionally mirrors protocol flags and values moved across async task boundaries"
)]
async fn query_dht_seed(
    local_addr: std::net::SocketAddr,
    local_keypair: AdnlKeyPair,
    remote: AdnlPublicKey,
    address: std::net::SocketAddr,
    node_count: i32,
    timeout: Duration,
) -> Option<Vec<DhtNode>> {
    let session = shared_session(local_addr, &local_keypair, remote, address)
        .await
        .ok()?;
    session
        .dht_find_node(Int256::random(), node_count, timeout)
        .await
        .ok()
        .map(|nodes| nodes.nodes)
}

pub fn direct_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: AdnlKeyPair,
) -> OverlaySessionFactory {
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
    local_keypair: AdnlKeyPair,
    timeout: Duration,
) -> OverlaySessionFactory {
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
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
) -> OverlaySessionFactory {
    overlay_factory_with_endpoints(
        local_addr,
        local_keypair,
        overlay,
        channel_timeout,
        OverlayMemberEndpoints::default(),
    )
}

/// [`overlay_factory`] that also shares one endpoint registry with every
/// session it spawns.
///
/// When a peer answers `overlay.getRandomPeers` on one of these sessions,
/// `inbound::trace_membership` records the session's source address
/// into `endpoints` - the route the transport actually sees, which follows
/// NAT rebinding - so the QUIC discovery path can probe the member at
/// `adnl_udp_port + 1000`.
pub fn overlay_factory_with_endpoints(
    local_addr: std::net::SocketAddr,
    local_keypair: AdnlKeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
    endpoints: OverlayMemberEndpoints,
) -> OverlaySessionFactory {
    build_overlay_factory(
        local_addr,
        &local_keypair,
        overlay,
        channel_timeout,
        endpoints,
        None,
    )
}

/// [`overlay_factory_with_endpoints`] that attaches one shared Plumtree
/// state handle to every session it spawns.
pub(crate) fn overlay_factory_with_endpoints_and_plumtree(
    local_addr: std::net::SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
    endpoints: OverlayMemberEndpoints,
    plumtree: PlumtreeHandle,
) -> OverlaySessionFactory {
    build_overlay_factory(
        local_addr,
        local_keypair,
        overlay,
        channel_timeout,
        endpoints,
        Some(plumtree),
    )
}

fn build_overlay_factory(
    local_addr: std::net::SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    channel_timeout: Option<Duration>,
    endpoints: OverlayMemberEndpoints,
    plumtree: Option<PlumtreeHandle>,
) -> OverlaySessionFactory {
    // One cache for every session this factory spawns: a member admitted
    // through one peer becomes gossip for all of them, which is what keeps
    // answers to `overlay.getRandomPeers` from collapsing to this node alone.
    let shared = OverlayMemberCache::default();
    // One pre-dedup per factory as well: flood copies of one broadcast
    // arrive from different peers, and the first sighting of a payload
    // hash suppresses the rest.
    let pre_dedup = PacketPreDedup::default();
    let local_keypair = *local_keypair;
    Arc::new(move |seed: SeedPeer| {
        let remote = AdnlPublicKey::from_bytes(seed.peer.as_bytes());
        let shared = shared.clone();
        let endpoints = Arc::clone(&endpoints);
        let pre_dedup = pre_dedup.clone();
        let plumtree = plumtree.clone();
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
            session.endpoints = endpoints;
            session.pre_dedup = pre_dedup;
            session.plumtree = plumtree;
            Ok(Box::new(session) as Box<dyn OverlaySession>)
        })
    })
}
