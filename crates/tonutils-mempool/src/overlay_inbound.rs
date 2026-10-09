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

use tl_proto::TlRead;
use tonutils_adnl::{AdnlUdpSession, now_i32};
use tonutils_overlay::{OverlayId, PeerId};
use tonutils_tl::tl::TonNodeCapabilities;
use tonutils_tl::tl::network::{
    OverlayMemberCertificate, OverlayNode, OverlayNodeV2, OverlayNodesBoxed, OverlayNodesV2Boxed,
    OverlayPong, OverlayQuery,
};

use crate::protocol_stats;
use crate::udp_session::valid_overlay_node;

/// Members this process learned from `overlay.getRandomPeers` answers.
///
/// Shared by every overlay session created from one factory so that a node
/// admitted through one peer is gossiped through all of them.  A plain
/// [`Mutex`] is enough: the guard is never held across an `.await`.
pub(crate) type OverlayMemberCache = Arc<Mutex<Vec<OverlayNode>>>;

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
    let parsed = match OverlayQuery::read_from(&mut data) {
        Ok(OverlayQuery::Query { overlay: wrapper })
        | Ok(OverlayQuery::QueryWithExtra {
            overlay: wrapper, ..
        }) => {
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
            OverlayQuery::read_from(&mut data)
        }
        other => other,
    };
    match parsed {
        Ok(OverlayQuery::Ping) => {
            log::debug!("answering overlay.ping for peer={peer:?}");
            protocol_stats::record_pong_sent();
            Some(tl_proto::serialize(OverlayPong))
        }
        Ok(OverlayQuery::GetCapabilities) => {
            log::debug!("answering tonNode.getCapabilities for peer={peer:?}");
            protocol_stats::record_capabilities_answer();
            Some(tl_proto::serialize(TonNodeCapabilities::current()))
        }
        Ok(OverlayQuery::GetRandomPeers { .. }) => {
            let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
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
            let ours = session.local_overlay_node_v2(tonutils_tl::Int256(overlay.as_bytes()));
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
        other => {
            protocol_stats::record_query_unhandled(effective_id);
            log::debug!(
                "unhandled overlay query: peer={peer:?} id={effective_id:08x} wrapped={wrapped} len={} recognised={}",
                query.len(),
                other.is_ok(),
            );
            None
        }
    }
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
    let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
    let listed = nodes.nodes.iter().any(|node| node.id == ours.id);
    protocol_stats::record_membership_answer(listed);
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
mod tests {
    use super::*;
    use tonutils_adnl::KeyPair;
    use tonutils_tl::tl::network::{OverlayMessageExtra, OverlayNodeToSign};

    const OVERLAY_PONG_WIRE: [u8; 4] = [0x81, 0xb4, 0x0c, 0x69];
    const OVERLAY_QUERY_WIRE: [u8; 4] = [0x43, 0x84, 0xfd, 0xcc];
    const OVERLAY_QUERY_WITH_EXTRA_WIRE: [u8; 4] = [0xe9, 0xc3, 0xff, 0x94];

    async fn test_session() -> AdnlUdpSession {
        let local = KeyPair::generate(&mut rand::rngs::OsRng);
        let remote = KeyPair::generate(&mut rand::rngs::OsRng);
        let local_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let remote_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
            .await
            .unwrap()
    }

    /// Builds the upstream wire framing: the `overlay.query` header followed
    /// by the raw inner query (`create_serialize_tl_object_suffix`).
    fn wrap(overlay: OverlayId, inner: &[u8]) -> Vec<u8> {
        let mut framed = tl_proto::serialize(OverlayQuery::Query {
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
        });
        framed.extend_from_slice(inner);
        framed
    }

    /// Fresh empty member cache for one test.
    fn cache() -> OverlayMemberCache {
        OverlayMemberCache::default()
    }

    /// Builds a `overlay.node` record signed by `key`, valid for `overlay`.
    ///
    /// `version` defaults to now; pass an explicit value to age a record out
    /// of the upstream `overlay_peer_ttl` window.
    fn signed_node(key: &KeyPair, overlay: OverlayId, version: Option<i32>) -> OverlayNode {
        let version = version.unwrap_or_else(now_i32);
        let public_key = tonutils_tl::tl::network::PublicKey::Ed25519 {
            key: tonutils_tl::Int256(key.public_key.to_bytes()),
        };
        let adnl_id = tonutils_adnl::AdnlAddress::from(&key.public_key).to_bytes();
        let signature = key.sign_raw(&tl_proto::serialize(OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: tonutils_tl::Int256(adnl_id),
            },
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
            version,
        }));
        OverlayNode {
            id: public_key,
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
            version,
            signature: signature.to_vec(),
        }
    }

    #[tokio::test]
    async fn answers_wrapped_overlay_ping_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([1; 32]);
        let ping = tl_proto::serialize(OverlayQuery::Ping);
        let wrapped = wrap(overlay, &ping);
        assert_eq!(&wrapped[..4], &OVERLAY_QUERY_WIRE);
        assert_eq!(&wrapped[4..36], overlay.as_bytes());
        assert_eq!(&wrapped[36..], &OVERLAY_PONG_WIRE);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped)
            .expect("wrapped ping must be answered");
        assert_eq!(&answer[..4], &[0x04, 0x08, 0x70, 0x67]);
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn answers_bare_overlay_ping_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([2; 32]);
        let ping = tl_proto::serialize(OverlayQuery::Ping);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &ping)
            .expect("bare ping must be answered");
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn answers_overlay_query_with_extra_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([3; 32]);
        let mut wrapped = tl_proto::serialize(OverlayQuery::QueryWithExtra {
            overlay: tonutils_tl::Int256(overlay.as_bytes()),
            extra: OverlayMessageExtra {
                flags: (),
                certificate: None,
            },
        });
        assert_eq!(&wrapped[..4], &OVERLAY_QUERY_WITH_EXTRA_WIRE);
        wrapped.extend_from_slice(&tl_proto::serialize(OverlayQuery::Ping));

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped)
            .expect("queryWithExtra ping must be answered");
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn rejects_overlay_query_wrapped_for_another_overlay() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let other = OverlayId::from_name(b"tonutils foreign overlay");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([4; 32]);
        let wrapped = wrap(other, &tl_proto::serialize(OverlayQuery::Ping));

        assert!(
            build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped).is_none(),
            "a query for a foreign overlay must not be answered"
        );
    }

    #[tokio::test]
    async fn answers_wrapped_get_random_peers_with_own_node() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([5; 32]);
        let query = tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: tonutils_tl::tl::network::OverlayNodes { nodes: Vec::new() },
        });
        let wrapped = wrap(overlay, &query);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped)
            .expect("getRandomPeers must be answered");
        let nodes: OverlayNodesBoxed = tl_proto::deserialize(&answer).expect("overlay.nodes");
        assert_eq!(nodes.nodes.len(), 1);
        assert_eq!(
            nodes.nodes[0].id,
            session
                .local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()))
                .id
        );
    }

    #[tokio::test]
    async fn answers_wrapped_get_random_peers_v2_with_own_node() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([6; 32]);
        let query = tl_proto::serialize(OverlayQuery::GetRandomPeersV2 {
            peers: tonutils_tl::tl::network::OverlayNodesV2 { nodes: Vec::new() },
        });
        let wrapped = wrap(overlay, &query);
        assert_eq!(&wrapped[..4], &[0x43, 0x84, 0xfd, 0xcc]);
        assert_eq!(&wrapped[36..40], &[0xcc, 0x7e, 0x8e, 0xa5]);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped)
            .expect("getRandomPeersV2 must be answered");
        assert_eq!(&answer[..4], &[0x42, 0x18, 0x07, 0xe4]);
        let nodes: OverlayNodesV2Boxed = tl_proto::deserialize(&answer).expect("overlay.nodesV2");
        assert_eq!(nodes.nodes.len(), 1);
        let ours = session.local_overlay_node_v2(tonutils_tl::Int256(overlay.as_bytes()));
        assert_eq!(nodes.nodes[0].id, ours.id);
        assert_eq!(nodes.nodes[0].flags, 0);
        assert_eq!(nodes.nodes[0].certificate, OverlayMemberCertificate::Empty);
        // A zero-flags record is signed over overlay.node.toSign, so
        // the signature must match the V1 record byte for byte.
        assert_eq!(
            nodes.nodes[0].signature,
            session
                .local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()))
                .signature
        );
    }

    #[tokio::test]
    async fn counts_membership_answer_listing_this_node() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([7; 32]);
        let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
        let answer = tl_proto::serialize(OverlayNodesBoxed { nodes: vec![ours] });
        let before = protocol_stats::protocol_stats();

        trace_membership(&peer, &session, Some(overlay), &members, &answer);

        let after = protocol_stats::protocol_stats();
        assert!(after.membership_listed > before.membership_listed);
    }

    #[tokio::test]
    async fn gossips_members_learned_from_an_answer() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([8; 32]);
        let foreign = signed_node(&KeyPair::generate(&mut rand::rngs::OsRng), overlay, None);
        let answer = tl_proto::serialize(OverlayNodesBoxed {
            nodes: vec![foreign.clone()],
        });

        trace_membership(&peer, &session, Some(overlay), &members, &answer);
        // Duplicates and this node's own record must not grow the cache.
        trace_membership(&peer, &session, Some(overlay), &members, &answer);

        let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
        trace_membership(
            &peer,
            &session,
            Some(overlay),
            &members,
            &tl_proto::serialize(OverlayNodesBoxed {
                nodes: vec![ours.clone()],
            }),
        );
        assert_eq!(
            members.lock().unwrap().len(),
            1,
            "only the foreign member belongs in the cache"
        );

        let query = tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: tonutils_tl::tl::network::OverlayNodes { nodes: Vec::new() },
        });
        let answer = build_overlay_answer(
            &peer,
            &session,
            Some(overlay),
            &members,
            &wrap(overlay, &query),
        )
        .expect("getRandomPeers must be answered");
        let nodes: OverlayNodesBoxed = tl_proto::deserialize(&answer).expect("overlay.nodes");
        assert_eq!(nodes.nodes.len(), 2, "self plus the learned member");
        assert_eq!(nodes.nodes[0].id, ours.id, "this node must come first");
        assert_eq!(nodes.nodes[1].id, foreign.id);
    }

    #[tokio::test]
    async fn drops_expired_and_foreign_members_before_gossiping() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([9; 32]);
        // Signed for this overlay but already past the 600 second peer TTL.
        let stale = signed_node(
            &KeyPair::generate(&mut rand::rngs::OsRng),
            overlay,
            Some(now_i32() - 1_100),
        );
        let foreign_overlay = signed_node(
            &KeyPair::generate(&mut rand::rngs::OsRng),
            OverlayId::from_name(b"tonutils foreign overlay"),
            None,
        );
        let answer = tl_proto::serialize(OverlayNodesBoxed {
            nodes: vec![stale, foreign_overlay],
        });

        trace_membership(&peer, &session, Some(overlay), &members, &answer);

        assert!(
            members.lock().unwrap().is_empty(),
            "expired and foreign records must never enter the cache"
        );
    }

    #[tokio::test]
    async fn reports_unhandled_and_session_without_overlay() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let members = cache();
        let peer = PeerId::from_bytes([6; 32]);
        let before = protocol_stats::protocol_stats();

        assert!(
            build_overlay_answer(
                &peer,
                &session,
                Some(overlay),
                &members,
                &[0x01, 0x02, 0x03, 0x04]
            )
            .is_none()
        );
        assert!(
            build_overlay_answer(
                &peer,
                &session,
                None,
                &members,
                &tl_proto::serialize(OverlayQuery::Ping)
            )
            .is_none()
        );

        let after = protocol_stats::protocol_stats();
        assert!(
            after.queries_unhandled > before.queries_unhandled,
            "unknown constructor ids must be counted as unhandled"
        );
        // Counters are process-wide, so only monotonic growth is asserted: other
        // tests running in parallel may answer queries at the same time.
        assert!(
            after.queries_received > before.queries_received,
            "both queries must be counted as received"
        );
    }
}
