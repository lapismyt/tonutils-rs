//! Incoming overlay protocol messages.
//!
//! Two kinds of inbound overlay traffic are understood here:
//!
//! * queries wrapped in the `overlay.query` header (see [`build_overlay_answer`]);
//! * `overlay.getRandomPeers` answers, whose membership list is traced by
//!   [`trace_membership`].
//!
//! The mainnet shard overlay is a full-node overlay, so a peer that has just
//! learned about this node also probes it with `tonNode.getCapabilities`
//! before treating it as a peer worth talking to
//! (`FullNodeQueries::process_query` in `validator/full-node-queries.hpp`).
//! Answering it costs one constructor and is the difference between a node
//! that looks alive on the wire and one that answers `unknown query`.
//!
//! A peer that received our `overlay.getRandomPeers` puts this node into a
//! pending set and then verifies it with `overlay.ping`
//! (`OverlayImpl::process_pending_peer` in `overlay/overlay-peers.cpp`). Only
//! a node that answers `overlay.pong` is promoted to a verified member, and
//! only verified members receive overlay broadcasts, so an unanswered ping
//! leaves the node connected but silent.
//!
//! The pytoniq nodes that make up the mempool overlay keep their peers alive
//! with a different constructor: `Node.ping` sends `dht.ping` wrapped in
//! `overlay.query` every 60 seconds and removes a peer after four unanswered
//! pings (`pytoniq/adnl/adnl.py`), long before any broadcast fan-out would
//! reach it. [`build_overlay_answer`] therefore answers `dht.ping` with
//! `dht.pong` as well, echoing `random_id` the way upstream
//! `DhtMemberImpl::process_query(dht_ping)` does (`dht/dht.cpp`).
//!
//! Every query arrives wrapped: upstream serializes queries with
//! `create_serialize_tl_object_suffix` (`OverlayManager::send_query` in
//! `overlay/overlay-manager.cpp`), i.e. it prefixes the inner query with
//! `overlay.query overlay:int256` — plus `overlay.queryWithExtra` when the
//! sending node carries a member certificate — and the receiver strips that
//! header first (`fetch_tl_prefix<overlay_query>`) before dispatching the
//! remainder to `OverlayImpl::process_query`.
//!
//! Answers to `overlay.getRandomPeers` are not limited to this node: every
//! verified member harvested by [`trace_membership`] is kept in a shared
//! [`OverlayMemberCache`] and gossiped back by [`build_overlay_answer`], the
//! way upstream answers with a subset of `nodes_`
//! (`OverlayImpl::send_random_peers`).

use std::sync::{Arc, Mutex};

use crate::{
    adnl::{
        AdnlUdpSession, now_i32,
        tl::{DhtMessage, DhtPong},
    },
    overlay::{
        OverlayId, OverlayMemberEndpoints, PeerId, PlumtreeHandle, protocol_stats, record_member,
        tl::{
            OverlayMemberCertificate, OverlayNode, OverlayNodeV2, OverlayNodesBoxed,
            OverlayNodesV2Boxed, OverlayPong, OverlayQuery, OverlayRepairPlumtreePart,
            TonNodeCapabilities,
        },
        udp_session::valid_overlay_node,
    },
    tl::TlInt256 as Int256,
};
use tl_proto::TlRead;

/// Members this process learned from `overlay.getRandomPeers` answers.
///
/// Shared by every overlay session created from one factory so that a node
/// admitted through one peer is gossiped through all of them.  A plain
/// [`Mutex`] is enough: the guard is never held across an `.await`.
pub type OverlayMemberCache = Arc<Mutex<Vec<OverlayNode>>>;

/// Upper bound on cached members.  Matches pytoniq's `OverlayManager(max_peers
/// = 30)`; older entries are dropped first so the freshest membership wins.
const MAX_CACHED_MEMBERS: usize = 30;

/// Members returned next to this node's own record.
///
/// Upstream sends up to `nodes_to_send_ = 8` nodes per answer
/// (`OverlayImpl::send_random_peers`); one slot stays reserved for this node,
/// so five borrowed members keep the answer comfortably inside that size.
const GOSSIP_MEMBER_COUNT: usize = 5;

/// Builds the answer for one incoming overlay query, or `None` when the query
/// is not recognised, is wrapped for a foreign overlay, or arrived on a
/// session that did not join an overlay.
///
/// Returns the serialized answer bytes; the caller decides how to deliver
/// them (an ADNL answer on the original query id).
#[allow(clippy::similar_names)]
pub(crate) fn build_overlay_answer(
    peer: &PeerId,
    session: &AdnlUdpSession,
    overlay: Option<OverlayId>,
    members: &OverlayMemberCache,
    query: &[u8],
) -> Option<Vec<u8>> {
    let Some(overlay) = overlay else {
        log::debug!(
            "overlay query on a session without overlay: peer={peer:?} id={:08x}",
            tl_id(query),
        );
        return None;
    };
    protocol_stats::record_query_received();
    let mut data = query;
    let mut effective_id = tl_id(query);
    let mut wrapped = false;
    // Start of the inner query, kept for constructors that are not part of
    // `OverlayQuery`: pytoniq wraps its `dht.ping` keepalive in
    // `overlay.query` (`OverlayTransport.send_query_message`), so the ping
    // arrives here with a valid wrapper but an inner constructor this
    // function must match itself.
    let mut inner = query;
    let parsed = match OverlayQuery::read_from(&mut data) {
        Ok(
            OverlayQuery::Query { overlay: wrapper }
            | OverlayQuery::QueryWithExtra {
                overlay: wrapper, ..
            },
        ) => {
            wrapped = true;
            effective_id = tl_id(data);
            protocol_stats::record_query_wrapped();
            if wrapper.0 != overlay.as_bytes() {
                protocol_stats::record_query_rejected();
                log::debug!(
                    "overlay query wrapped for another overlay: peer={peer:?} joined={:02x?} wrapped={:02x?}",
                    overlay.as_bytes(),
                    wrapper.0,
                );
                return None;
            }
            inner = data;
            OverlayQuery::read_from(&mut data)
        }
        other => other,
    };
    match parsed {
        Ok(OverlayQuery::Ping) => {
            log::debug!("answering overlay.ping for peer={peer:?}");
            protocol_stats::record_pong_sent();
            protocol_stats::record_ping_from(peer.as_bytes());
            Some(tl_proto::serialize(OverlayPong))
        }
        Ok(OverlayQuery::GetCapabilities) => {
            log::debug!("answering tonNode.getCapabilities for peer={peer:?}");
            protocol_stats::record_capabilities_answer();
            Some(tl_proto::serialize(TonNodeCapabilities::current()))
        }
        Ok(OverlayQuery::GetRandomPeers { .. }) => {
            let ours = session.local_overlay_node(Int256(overlay.as_bytes()));
            let mut nodes = vec![ours.clone()];
            nodes.extend(gossip_members(members, overlay, &ours));
            let nodes = OverlayNodesBoxed { nodes };
            log::debug!(
                "answering overlay.getRandomPeers for peer={peer:?} with {} member(s)",
                nodes.nodes.len()
            );
            protocol_stats::record_random_peers_answer();
            Some(tl_proto::serialize(nodes))
        }
        Ok(OverlayQuery::GetRandomPeersV2 { .. }) => {
            let ours = session.local_overlay_node_v2(Int256(overlay.as_bytes()));
            let mut nodes = vec![ours.clone()];
            nodes.extend(gossip_members_v2(members, overlay, &ours));
            let nodes = OverlayNodesV2Boxed { nodes };
            log::debug!(
                "answering overlay.getRandomPeersV2 for peer={peer:?} with {} member(s)",
                nodes.nodes.len()
            );
            protocol_stats::record_random_peers_answer();
            Some(tl_proto::serialize(nodes))
        }
        // `Err` is a constructor this enum does not model — `dht.ping` is
        // the one that matters in practice; a nested `overlay.query` is a
        // double-wrapped query and equally unanswerable.
        Ok(
            OverlayQuery::Query { .. }
            | OverlayQuery::QueryWithExtra { .. }
            | OverlayQuery::RepairPlumtreePart { .. },
        )
        | Err(_) => {
            // pytoniq pings its overlay peers with `dht.ping` wrapped in
            // `overlay.query` and removes a peer after four unanswered
            // pings (`Node.ping` in `pytoniq/adnl/adnl.py`), so silence
            // here silently drops this node from the peer's broadcast
            // fan-out.  Upstream answers the same constructor from the
            // DHT member (`DhtMemberImpl::process_query(dht_ping)`).
            let mut dht = inner;
            if let Ok(DhtMessage::Ping { random_id }) = DhtMessage::read_from(&mut dht) {
                log::debug!("answering dht.ping for peer={peer:?}");
                protocol_stats::record_pong_sent();
                protocol_stats::record_dht_ping();
                return Some(tl_proto::serialize(DhtPong { random_id }));
            }
            protocol_stats::record_query_unhandled(effective_id);
            log::debug!(
                "unhandled overlay query: peer={peer:?} id={effective_id:08x} wrapped={wrapped} len={}",
                query.len(),
            );
            None
        }
    }
}

/// Parses an `overlay.repairPlumtreePart` query, with or without the
/// `overlay.query{overlay}` wrapper.
pub(crate) fn parse_repair_query(
    query: &[u8],
    overlay: OverlayId,
) -> Option<OverlayRepairPlumtreePart> {
    let mut data = query;
    let parsed = match OverlayQuery::read_from(&mut data) {
        Ok(
            OverlayQuery::Query { overlay: wrapper }
            | OverlayQuery::QueryWithExtra {
                overlay: wrapper, ..
            },
        ) => {
            if wrapper.0 != overlay.as_bytes() {
                return None;
            }
            OverlayQuery::read_from(&mut data)
        }
        other => other,
    };
    match parsed {
        Ok(OverlayQuery::RepairPlumtreePart {
            broadcast_id,
            timestamp,
            part_index,
            tree_index,
        }) => Some(OverlayRepairPlumtreePart {
            broadcast_id,
            timestamp,
            part_index,
            tree_index,
        }),
        _ => None,
    }
}

/// Dispatches an `overlay.repairPlumtreePart` query to the shared Plumtree
/// state.
///
/// Returns `None` when the query is not a repair query, `Some(None)` when it
/// is a repair query this node cannot answer, and `Some(Some(answer))` when
/// the part is available. The caller only owns transport framing and the
/// answer write; repair validation, quota accounting, and statistics stay in
/// one place for UDP and QUIC.
pub(crate) async fn answer_repair_query(
    peer: &PeerId,
    overlay: OverlayId,
    plumtree: Option<&PlumtreeHandle>,
    query: &[u8],
) -> Option<Option<Vec<u8>>> {
    let part = parse_repair_query(query, overlay)?;
    let Some(plumtree) = plumtree else {
        return Some(None);
    };
    protocol_stats::record_query_received();
    match plumtree
        .answer_repair(
            *peer,
            part.broadcast_id,
            part.timestamp.to_f64(),
            part.part_index,
            part.tree_index,
        )
        .await
    {
        Ok(answer) => Some(answer),
        Err(error) => {
            protocol_stats::record_query_answer_failed();
            log::debug!("overlay repair query rejected: peer={peer:?} error={error}");
            Some(None)
        }
    }
}

/// Constructor words the synchronous fast path covers: the two
/// `overlay.query` wrappers plus the two bare keepalive pings.
///
/// These are exactly the queries whose answers need no shared state, so
/// the session's receive task can send them without the consumer hop
/// that decides whether a peer keeps this node alive (see
/// [`build_fast_answer`]).
pub(crate) const FAST_ANSWER_IDS: [u32; 4] = [
    OVERLAY_QUERY_ID,
    OVERLAY_QUERY_WITH_EXTRA_ID,
    OVERLAY_PING_ID,
    DHT_PING_ID,
];

/// `overlay.query` constructor word.
const OVERLAY_QUERY_ID: u32 = 0xccfd8443;
/// `overlay.queryWithExtra` constructor word.
const OVERLAY_QUERY_WITH_EXTRA_ID: u32 = 0x94ffc3e9;
/// `overlay.ping` constructor word.
const OVERLAY_PING_ID: u32 = 0x690cb481;
/// `dht.ping` constructor word.
const DHT_PING_ID: u32 = 0xcbeb3f18;

/// Builds the synchronous answer registered on one overlay session.
///
/// Covers exactly the queries [`build_overlay_answer`] answers from no
/// shared state: `overlay.ping` and `dht.ping`, bare or wrapped in
/// `overlay.query`/`overlay.queryWithExtra`.  These are the liveness
/// probes a peer measures against its own deadline
/// (`OverlayImpl::process_pending_peer`), and the consumer loop can be
/// busy with broadcast work for longer than that deadline, so the
/// session's receive task answers them directly; run 39114243866
/// measured second-scale in-process answer latencies while every peer
/// probe went unanswered.
///
/// The closure declines (`None`) everything else - including a wrapper
/// for a foreign overlay - without touching the protocol counters, so
/// the consumer's own dispatch records those queries exactly once, as
/// before, and the counter parity of the inbound query census holds.
/// The `overlay.ping` and `dht.ping` branches bump exactly the counters
/// the consumer path bumps for the same probes.
pub(crate) fn build_fast_answer(
    overlay: OverlayId,
    peer: PeerId,
) -> impl Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync + 'static {
    move |query: &[u8]| {
        let mut data = query;
        match tl_id(query) {
            OVERLAY_QUERY_ID | OVERLAY_QUERY_WITH_EXTRA_ID => {
                let Ok(
                    OverlayQuery::Query { overlay: wrapper }
                    | OverlayQuery::QueryWithExtra {
                        overlay: wrapper, ..
                    },
                ) = OverlayQuery::read_from(&mut data)
                else {
                    return None;
                };
                if wrapper.0 != overlay.as_bytes() {
                    return None;
                }
                match tl_id(data) {
                    OVERLAY_PING_ID => {
                        protocol_stats::record_query_received();
                        protocol_stats::record_query_wrapped();
                        Some(answer_overlay_ping(&peer))
                    }
                    DHT_PING_ID => {
                        protocol_stats::record_query_received();
                        protocol_stats::record_query_wrapped();
                        answer_dht_ping(data)
                    }
                    _ => None,
                }
            }
            OVERLAY_PING_ID => {
                protocol_stats::record_query_received();
                Some(answer_overlay_ping(&peer))
            }
            DHT_PING_ID => {
                protocol_stats::record_query_received();
                answer_dht_ping(query)
            }
            _ => None,
        }
    }
}

/// Counts one answered `overlay.ping` and builds its `overlay.pong`.
fn answer_overlay_ping(peer: &PeerId) -> Vec<u8> {
    protocol_stats::record_pong_sent();
    protocol_stats::record_ping_from(peer.as_bytes());
    tl_proto::serialize(OverlayPong)
}

/// Counts one answered `dht.ping` and builds its echoing `dht.pong`.
fn answer_dht_ping(query: &[u8]) -> Option<Vec<u8>> {
    let mut data = query;
    let Ok(DhtMessage::Ping { random_id }) = DhtMessage::read_from(&mut data) else {
        return None;
    };
    protocol_stats::record_pong_sent();
    protocol_stats::record_dht_ping();
    Some(tl_proto::serialize(DhtPong { random_id }))
}

/// Logs whether a peer lists this node among the overlay members it returns
/// from `overlay.getRandomPeers`.
///
/// Upstream only answers with *verified* members
/// (`OverlayImpl::send_random_peers`), so this is the observable signal that
/// the peer completed `overlay.ping`/`overlay.pong` and now treats this node
/// as a peer that may receive broadcasts.
pub(crate) fn trace_membership(
    peer: &PeerId,
    session: &AdnlUdpSession,
    overlay: Option<OverlayId>,
    members: &OverlayMemberCache,
    endpoints: &OverlayMemberEndpoints,
    answer: &[u8],
) {
    let Some(overlay) = overlay else {
        return;
    };
    let mut data = answer;
    let Ok(nodes) = OverlayNodesBoxed::read_from(&mut data) else {
        log::debug!("overlay getRandomPeers answer: peer={peer:?} (not overlay.nodes)");
        return;
    };
    // The peer answered our `overlay.getRandomPeers` with a parseable
    // membership list holding at least one record valid for this overlay,
    // which is the same evidence the growth path uses to treat a peer as a
    // member, so its session source address becomes a QUIC probe target
    // (`adnl_udp_port + 1000` upstream).  `remote_addr` is the route the
    // transport actually sees, which follows NAT rebinding through
    // `adopt_address` - unlike the address this node dialled, which is
    // recorded elsewhere from the peer's DHT `address` record.
    let now = now_i32();
    let valid_members = nodes
        .nodes
        .iter()
        .filter(|node| valid_overlay_node(node, overlay, now))
        .count();
    if valid_members > 0 {
        record_member(endpoints, *peer, session.remote_addr());
    }
    let ours = session.local_overlay_node(Int256(overlay.as_bytes()));
    let listed = nodes.nodes.iter().any(|node| node.id == ours.id);
    protocol_stats::record_membership_answer(listed, peer.as_bytes());
    log::info!(
        "overlay getRandomPeers answer: peer={peer:?} nodes={} listed_us={listed}",
        nodes.nodes.len()
    );
    let added = remember_members(members, overlay, &ours, nodes.nodes);
    if added > 0 {
        log::debug!("overlay member cache: learned {added} member(s) from {peer:?}");
    }
}

/// Keeps the verified members a peer returned, dropping this node's own record
/// and anything that fails [`valid_overlay_node`] for `overlay`.
///
/// Returns how many members were newly inserted.
fn remember_members(
    members: &OverlayMemberCache,
    overlay: OverlayId,
    ours: &OverlayNode,
    learned: Vec<OverlayNode>,
) -> usize {
    let now = now_i32();
    let Ok(mut cache) = members.lock() else {
        return 0;
    };
    let before = cache.len();
    for node in learned {
        if node.id == ours.id || !valid_overlay_node(&node, overlay, now) {
            continue;
        }
        if cache.iter().any(|known| known.id == node.id) {
            continue;
        }
        cache.push(node);
    }
    if cache.len() > MAX_CACHED_MEMBERS {
        let excess = cache.len() - MAX_CACHED_MEMBERS;
        cache.drain(..excess);
    }
    cache.len() - before
}

/// Picks up to [`GOSSIP_MEMBER_COUNT`] cached members to return next to `ours`.
///
/// Entries are re-validated against the current clock: a cached record that
/// aged past the upstream `overlay_peer_ttl` of 600 seconds stops being
/// advertised instead of being handed out forever.
fn gossip_members(
    members: &OverlayMemberCache,
    overlay: OverlayId,
    ours: &OverlayNode,
) -> Vec<OverlayNode> {
    let now = now_i32();
    let Ok(mut cache) = members.lock() else {
        return Vec::new();
    };
    cache.retain(|node| valid_overlay_node(node, overlay, now));
    cache
        .iter()
        .filter(|node| node.id != ours.id)
        .take(GOSSIP_MEMBER_COUNT)
        .cloned()
        .collect()
}

/// Picks up to [`GOSSIP_MEMBER_COUNT`] cached members as `overlay.nodeV2`
/// records for a `getRandomPeersV2` answer.
///
/// The cache holds V1 records; upstream converts its peer records with
/// `OverlayNode::tl_v2` when answering V2, and every record converts
/// because the V2 form only adds the `flags` and `certificate` fields,
/// both of which stay at their empty defaults here.
fn gossip_members_v2(
    members: &OverlayMemberCache,
    overlay: OverlayId,
    ours: &OverlayNodeV2,
) -> Vec<OverlayNodeV2> {
    let now = now_i32();
    let Ok(mut cache) = members.lock() else {
        return Vec::new();
    };
    cache.retain(|node| valid_overlay_node(node, overlay, now));
    cache
        .iter()
        .filter(|node| node.id != ours.id)
        .take(GOSSIP_MEMBER_COUNT)
        .map(|node| OverlayNodeV2 {
            id: node.id.clone(),
            overlay: node.overlay.clone(),
            flags: 0,
            version: node.version,
            signature: node.signature.clone(),
            certificate: OverlayMemberCertificate::Empty,
        })
        .collect()
}

/// Reads the little-endian TL constructor id of a serialized object.
fn tl_id(data: &[u8]) -> u32 {
    data.get(..4)
        .map_or(0, |id| u32::from_le_bytes([id[0], id[1], id[2], id[3]]))
}

#[cfg(test)]
mod tests;
