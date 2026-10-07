//! Incoming overlay protocol messages.
//!
//! Two kinds of inbound overlay traffic are understood here:
//!
//! * queries wrapped in the `overlay.query` header (see [`build_overlay_answer`]);
//! * `overlay.getRandomPeers` answers, whose membership list is traced by
//!   [`trace_membership`].
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

use tl_proto::TlRead;
use tonutils_adnl::AdnlUdpSession;
use tonutils_overlay::{OverlayId, PeerId};
use tonutils_tl::tl::network::{OverlayNodesBoxed, OverlayPong, OverlayQuery};

use crate::protocol_stats;

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
        Ok(OverlayQuery::GetRandomPeers { .. }) => {
            let nodes = OverlayNodesBoxed {
                nodes: vec![session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()))],
            };
            log::debug!("answering overlay.getRandomPeers for peer={peer:?}");
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
    use tonutils_tl::tl::network::OverlayMessageExtra;

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

    #[tokio::test]
    async fn answers_wrapped_overlay_ping_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let peer = PeerId::from_bytes([1; 32]);
        let ping = tl_proto::serialize(OverlayQuery::Ping);
        let wrapped = wrap(overlay, &ping);
        assert_eq!(&wrapped[..4], &OVERLAY_QUERY_WIRE);
        assert_eq!(&wrapped[4..36], overlay.as_bytes());
        assert_eq!(&wrapped[36..], &OVERLAY_PONG_WIRE);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &wrapped)
            .expect("wrapped ping must be answered");
        assert_eq!(&answer[..4], &[0x04, 0x08, 0x70, 0x67]);
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn answers_bare_overlay_ping_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let peer = PeerId::from_bytes([2; 32]);
        let ping = tl_proto::serialize(OverlayQuery::Ping);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &ping)
            .expect("bare ping must be answered");
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn answers_overlay_query_with_extra_with_pong() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
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

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &wrapped)
            .expect("queryWithExtra ping must be answered");
        assert_eq!(answer, tl_proto::serialize(OverlayPong));
    }

    #[tokio::test]
    async fn rejects_overlay_query_wrapped_for_another_overlay() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let other = OverlayId::from_name(b"tonutils foreign overlay");
        let session = test_session().await;
        let peer = PeerId::from_bytes([4; 32]);
        let wrapped = wrap(other, &tl_proto::serialize(OverlayQuery::Ping));

        assert!(
            build_overlay_answer(&peer, &session, Some(overlay), &wrapped).is_none(),
            "a query for a foreign overlay must not be answered"
        );
    }

    #[tokio::test]
    async fn answers_wrapped_get_random_peers_with_own_node() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let peer = PeerId::from_bytes([5; 32]);
        let query = tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: tonutils_tl::tl::network::OverlayNodes { nodes: Vec::new() },
        });
        let wrapped = wrap(overlay, &query);

        let answer = build_overlay_answer(&peer, &session, Some(overlay), &wrapped)
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
    async fn counts_membership_answer_listing_this_node() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let peer = PeerId::from_bytes([7; 32]);
        let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
        let answer = tl_proto::serialize(OverlayNodesBoxed { nodes: vec![ours] });
        let before = protocol_stats::protocol_stats();

        trace_membership(&peer, &session, Some(overlay), &answer);

        let after = protocol_stats::protocol_stats();
        assert!(after.membership_listed > before.membership_listed);
    }

    #[tokio::test]
    async fn reports_unhandled_and_session_without_overlay() {
        let overlay = OverlayId::from_name(b"tonutils query test");
        let session = test_session().await;
        let peer = PeerId::from_bytes([6; 32]);
        let before = protocol_stats::protocol_stats();

        assert!(
            build_overlay_answer(&peer, &session, Some(overlay), &[0x01, 0x02, 0x03, 0x04])
                .is_none()
        );
        assert!(
            build_overlay_answer(
                &peer,
                &session,
                None,
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
