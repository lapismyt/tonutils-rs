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

/// Fresh empty member endpoint registry for one test.
fn endpoints() -> crate::OverlayMemberEndpoints {
    crate::OverlayMemberEndpoints::default()
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

/// Distinct pingers and `dht.ping` probes are the two observable admission
/// signals: an `overlay.ping` arrives only from a peer that admitted this
/// node, and pytoniq-style `dht.ping` probes arrive only from peers that
/// keep it in their own peer list.  Repeat queries from one peer may not
/// inflate the distinct-pinger figure.
#[tokio::test]
async fn counts_distinct_pingers_and_dht_ping_probes() {
    let overlay = OverlayId::from_name(b"tonutils query test");
    let session = test_session().await;
    let members = cache();
    let peer = PeerId::from_bytes([42; 32]);
    let ping = tl_proto::serialize(OverlayQuery::Ping);
    let before = protocol_stats::protocol_stats();

    for _ in 0..3 {
        build_overlay_answer(&peer, &session, Some(overlay), &members, &ping)
            .expect("repeat ping must be answered");
    }
    let dht_ping = tl_proto::serialize(DhtMessage::Ping { random_id: 7 });
    let wrapped_dht = wrap(overlay, &dht_ping);
    build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped_dht)
        .expect("wrapped dht.ping must be answered");

    let after = protocol_stats::protocol_stats();
    // The counters are process-global and the sibling query tests run
    // in parallel, so the deltas are lower bounds: this test adds one
    // distinct pinger and one dht.ping probe, others may add more.
    assert!(
        after.unique_pingers - before.unique_pingers >= 1,
        "three pings from one peer must count at least one new distinct pinger"
    );
    assert!(
        after.dht_pings_received - before.dht_pings_received >= 1,
        "the dht.ping probe must be counted"
    );
}

#[tokio::test]
async fn answers_wrapped_dht_ping_with_pong() {
    let overlay = OverlayId::from_name(b"tonutils query test");
    let session = test_session().await;
    let members = cache();
    let peer = PeerId::from_bytes([4; 32]);
    // pytoniq's `Node.ping` shape: `dht.ping` wrapped in `overlay.query`,
    // random_id echoed back or the peer counts the ping as lost.
    let ping = tl_proto::serialize(DhtMessage::Ping { random_id: 42 });
    let wrapped = wrap(overlay, &ping);
    assert_eq!(&wrapped[..4], &OVERLAY_QUERY_WIRE);

    let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &wrapped)
        .expect("wrapped dht.ping must be answered");
    assert_eq!(&answer[..4], &[0x81, 0xef, 0x8a, 0x5a]);
    assert_eq!(answer, tl_proto::serialize(DhtPong { random_id: 42 }));
}

#[tokio::test]
async fn answers_bare_dht_ping_with_pong() {
    let overlay = OverlayId::from_name(b"tonutils query test");
    let session = test_session().await;
    let members = cache();
    let peer = PeerId::from_bytes([5; 32]);
    let ping = tl_proto::serialize(DhtMessage::Ping {
        random_id: u64::MAX,
    });

    let answer = build_overlay_answer(&peer, &session, Some(overlay), &members, &ping)
        .expect("bare dht.ping must be answered");
    assert_eq!(
        answer,
        tl_proto::serialize(DhtPong {
            random_id: u64::MAX
        })
    );
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
    let endpoints = endpoints();
    let peer = PeerId::from_bytes([7; 32]);
    let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
    let answer = tl_proto::serialize(OverlayNodesBoxed { nodes: vec![ours] });
    let before = protocol_stats::protocol_stats();

    trace_membership(
        &peer,
        &session,
        Some(overlay),
        &members,
        &endpoints,
        &answer,
    );

    let after = protocol_stats::protocol_stats();
    assert!(after.membership_listed > before.membership_listed);
    assert!(
        after.membership_listing_peers > before.membership_listing_peers,
        "the listing peer must be tracked distinctly"
    );
    // A well-formed answer also registers the answerer's session route as
    // a member endpoint for the QUIC probe registry.
    assert_eq!(crate::registered_member_count(&endpoints), 1);
    assert_eq!(
        crate::known_member_seeds(&endpoints),
        vec![(peer, session.remote_addr())],
        "the session source address must be the registered endpoint"
    );
}

#[tokio::test]
async fn gossips_members_learned_from_an_answer() {
    let overlay = OverlayId::from_name(b"tonutils query test");
    let session = test_session().await;
    let members = cache();
    let endpoints = endpoints();
    let peer = PeerId::from_bytes([8; 32]);
    let foreign = signed_node(&KeyPair::generate(&mut rand::rngs::OsRng), overlay, None);
    let answer = tl_proto::serialize(OverlayNodesBoxed {
        nodes: vec![foreign.clone()],
    });

    trace_membership(
        &peer,
        &session,
        Some(overlay),
        &members,
        &endpoints,
        &answer,
    );
    // Duplicates and this node's own record must not grow the cache.
    trace_membership(
        &peer,
        &session,
        Some(overlay),
        &members,
        &endpoints,
        &answer,
    );

    let ours = session.local_overlay_node(tonutils_tl::Int256(overlay.as_bytes()));
    trace_membership(
        &peer,
        &session,
        Some(overlay),
        &members,
        &endpoints,
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
    let endpoints = endpoints();
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

    trace_membership(
        &peer,
        &session,
        Some(overlay),
        &members,
        &endpoints,
        &answer,
    );

    assert!(
        members.lock().unwrap().is_empty(),
        "expired and foreign records must never enter the cache"
    );
    assert_eq!(
        crate::registered_member_count(&endpoints),
        0,
        "an answer without a single valid record is no membership evidence"
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

/// The synchronous fast answer must answer byte for byte what
/// [`build_overlay_answer`] answers from no shared state, and decline
/// everything else so the consumer's dispatch keeps owning those queries.
#[test]
fn fast_answer_mirrors_the_consumer_path_and_declines_the_rest() {
    let overlay = OverlayId::from_name(b"tonutils fast answer test");
    let foreign = OverlayId::from_name(b"tonutils foreign overlay");
    let peer = PeerId::from_bytes([9; 32]);
    let answer = build_fast_answer(overlay, peer);

    // `overlay.ping` answers with the constant pong, bare or wrapped in
    // either wrapper.
    let ping = tl_proto::serialize(OverlayQuery::Ping);
    let pong = tl_proto::serialize(OverlayPong);
    assert_eq!(
        answer(&ping),
        Some(pong.clone()),
        "bare overlay.ping must answer with a pong"
    );
    assert_eq!(
        answer(&wrap(overlay, &ping)),
        Some(pong.clone()),
        "wrapped overlay.ping must answer with a pong"
    );
    let mut with_extra = tl_proto::serialize(OverlayQuery::QueryWithExtra {
        overlay: tonutils_tl::Int256(overlay.as_bytes()),
        extra: OverlayMessageExtra {
            flags: (),
            certificate: None,
        },
    });
    with_extra.extend_from_slice(&ping);
    assert_eq!(
        answer(&with_extra),
        Some(pong),
        "queryWithExtra-wrapped overlay.ping must answer with a pong"
    );

    // `dht.ping` echoes its random id, bare or wrapped.
    let dht_ping = tl_proto::serialize(DhtMessage::Ping { random_id: 4242 });
    let dht_pong = tl_proto::serialize(DhtPong { random_id: 4242 });
    assert_eq!(
        answer(&dht_ping),
        Some(dht_pong.clone()),
        "bare dht.ping must echo its random id"
    );
    assert_eq!(
        answer(&wrap(overlay, &dht_ping)),
        Some(dht_pong),
        "wrapped dht.ping must echo its random id"
    );

    // Everything else stays with the consumer: a wrapper for a foreign
    // overlay, a query the consumer answers from shared state, a chunk
    // shorter than a constructor word, and a header without a payload.
    assert_eq!(
        answer(&wrap(foreign, &ping)),
        None,
        "a wrapper for a foreign overlay must decline"
    );
    let find_node = tl_proto::serialize(DhtMessage::FindNode {
        key: tonutils_tl::Int256([3; 32]),
        k: 8,
    });
    assert_eq!(
        answer(&wrap(overlay, &find_node)),
        None,
        "a query the consumer answers from shared state must decline"
    );
    assert_eq!(answer(&[0xde, 0xad, 0xbe]), None);
    assert_eq!(answer(&OVERLAY_QUERY_WIRE), None);
    assert_eq!(
        answer(&DHT_PING_ID.to_le_bytes()),
        None,
        "a dht.ping without its random id must decline"
    );
}

/// The fast path must keep the inbound query census: the answered probes
/// move the same counters the consumer's dispatch moved, so gate and run
/// analysis keep reading the same fields.
#[test]
fn fast_answer_counts_the_same_probes_as_the_consumer_path() {
    let overlay = OverlayId::from_name(b"tonutils fast answer test");
    let peer = PeerId::from_bytes([10; 32]);
    let answer = build_fast_answer(overlay, peer);
    let before = protocol_stats::protocol_stats();

    let ping = tl_proto::serialize(OverlayQuery::Ping);
    assert!(answer(&ping).is_some(), "bare overlay.ping must answer");
    assert!(
        answer(&wrap(overlay, &ping)).is_some(),
        "wrapped overlay.ping must answer"
    );
    assert!(
        answer(&tl_proto::serialize(DhtMessage::Ping { random_id: 1 })).is_some(),
        "bare dht.ping must answer"
    );
    assert!(
        answer(&wrap(
            overlay,
            &tl_proto::serialize(DhtMessage::Ping { random_id: 2 })
        ))
        .is_some(),
        "wrapped dht.ping must answer"
    );

    let after = protocol_stats::protocol_stats();
    // The counters are process-global and the sibling query tests run in
    // parallel, so the deltas are lower bounds: this test answers four
    // probes and other tests may answer more.
    assert!(
        after.queries_received - before.queries_received >= 4,
        "every answered probe must count as a received query"
    );
    assert!(
        after.queries_wrapped - before.queries_wrapped >= 2,
        "the wrapped probes must count as wrapped"
    );
    assert!(
        after.pongs_sent - before.pongs_sent >= 4,
        "every answered probe must count as a sent pong"
    );
    assert!(
        after.unique_pingers - before.unique_pingers >= 1,
        "the overlay.ping probes must count one distinct pinger"
    );
    assert!(
        after.dht_pings_received - before.dht_pings_received >= 2,
        "the dht.ping probes must count as dht pings"
    );

    // A declined query must not move anything: the consumer's dispatch
    // records it when the packet reaches the inbox, exactly once.
    let declined = wrap(
        OverlayId::from_name(b"tonutils foreign overlay"),
        &tl_proto::serialize(DhtMessage::Ping { random_id: 3 }),
    );
    assert_eq!(answer(&declined), None);
}
