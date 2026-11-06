//! Synchronous answers sent by a session's receive task.
//!
//! These pin the two halves of
//! [`AdnlUdpSession::register_query_answer`]: an answered query is served
//! by the receive task itself and never reaches the consumer, and a
//! registration that declines a query leaves it to the consumer
//! untouched.

use std::time::Duration;

use crate::{
    adnl::{
        AdnlError, AdnlKeyPair, AdnlUdpSession, AdnlUdpTransport,
        tl::{AdnlMessage, DhtMessage, DhtNodesBoxed, PacketContents},
    },
    tl::TlInt256 as Int256,
};

/// Two shared sessions, one on each side, routed to each other.
async fn shared_pair() -> (AdnlUdpSession, AdnlUdpSession) {
    let sender_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let receiver_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let sender_transport = AdnlUdpTransport::bind("127.0.0.1:0".parse().unwrap(), sender_key)
        .await
        .expect("sender transport must bind");
    let receiver_transport = AdnlUdpTransport::bind("127.0.0.1:0".parse().unwrap(), receiver_key)
        .await
        .expect("receiver transport must bind");
    let sender_addr = sender_transport
        .local_addr()
        .expect("sender transport address");
    let receiver_addr = receiver_transport
        .local_addr()
        .expect("receiver transport address");
    let receiver = receiver_transport.session_for_peer(sender_key.public_key, sender_addr);
    let sender = sender_transport.session_for_peer(receiver_key.public_key, receiver_addr);
    (sender, receiver)
}

/// The constructor word of `dht.findNode`, read off its own
/// serialization so the test cannot drift from the TL schema.
fn dht_find_node_id() -> u32 {
    let query = tl_proto::serialize(DhtMessage::FindNode {
        key: Int256([7; 32]),
        k: 8,
    });
    u32::from_le_bytes(query[..4].try_into().unwrap())
}

/// A query the receive task answers must be answered there: the
/// consumer's inbox never sees it, which is what lets a registered
/// answer keep a peer's liveness deadline when the consumer is busy.
#[tokio::test]
async fn registered_answer_serves_the_query_without_the_consumer() {
    let (sender, receiver) = shared_pair().await;
    receiver.register_query_answer(dht_find_node_id(), |_query| {
        Some(tl_proto::serialize(DhtNodesBoxed { nodes: Vec::new() }))
    });

    let answer = sender
        .dht_find_node(Int256([9; 32]), 8, Duration::from_secs(2))
        .await
        .expect("the registered answer must reach the query's waiter");
    assert!(
        answer.nodes.is_empty(),
        "the registered answer's empty node list must round-trip, got {:?}",
        answer.nodes
    );

    let stray = receiver.recv_timeout(Duration::from_millis(300)).await;
    assert!(
        matches!(stray, Err(AdnlError::Timeout { .. })),
        "an answered query must not reach the consumer, got {stray:?}"
    );
}

/// A registration that declines a query must leave it to the consumer
/// exactly as if nothing had been registered.
#[tokio::test]
async fn declined_query_takes_the_ordinary_consumer_path() {
    let (sender, receiver) = shared_pair().await;
    receiver.register_query_answer(0xdead_beef, |_query| None);

    let query_id = Int256::random();
    let mut query = 0xdead_beefu32.to_le_bytes().to_vec();
    query.extend_from_slice(b"declined payload");
    sender
        .send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id,
                query: query.clone(),
            }),
            messages: None,
            address: None,
            priority_address: None,
            seqno: None,
            confirm_seqno: None,
            recv_addr_list_version: None,
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: None,
            signature: None,
            rand2: vec![0; 7],
        })
        .await
        .expect("query must send");

    let packet = receiver
        .recv_timeout(Duration::from_secs(2))
        .await
        .expect("a declined query must reach the consumer");
    let Some(AdnlMessage::Query {
        query: received, ..
    }) = packet.message
    else {
        panic!("expected the declined query, got {packet:?}");
    };
    assert_eq!(received, query);
}

/// A full consumer inbox must not hold up the registered answers of
/// later datagrams: the receive loop keeps reading the session queue
/// while a decoded packet waits for inbox space, so a peer's liveness
/// probe is answered inside its five-second window even when the
/// consumer has stopped draining.  Packets the consumer is missing in
/// that state are dropped and counted instead of queued.
#[tokio::test]
async fn registered_answers_leave_while_the_consumer_inbox_is_full() {
    let (sender, receiver) = shared_pair().await;
    receiver.register_query_answer(dht_find_node_id(), |_query| {
        Some(tl_proto::serialize(DhtNodesBoxed { nodes: Vec::new() }))
    });

    // 64 consumer-bound queries fill the inbox (SESSION_QUEUE), the
    // next one parks inside the receive loop, and the one after it is
    // dropped and counted.  Nothing ever drains the inbox.
    let consumer_bound = || PacketContents {
        rand1: vec![0; 7],
        flags: (),
        from: None,
        from_short: None,
        message: Some(AdnlMessage::Query {
            query_id: Int256::random(),
            query: 0xdead_beefu32.to_le_bytes().to_vec(),
        }),
        messages: None,
        address: None,
        priority_address: None,
        seqno: None,
        confirm_seqno: None,
        recv_addr_list_version: None,
        recv_priority_addr_list_version: None,
        reinit_date: None,
        dst_reinit_date: None,
        signature: None,
        rand2: vec![0; 7],
    };
    for _ in 0..66 {
        sender
            .send_contents(consumer_bound())
            .await
            .expect("flood query must send");
    }

    // The registered query is dispatched while the loop waits for inbox
    // space, so the answer leaves although the consumer never receives.
    let answer = tokio::time::timeout(
        Duration::from_secs(3),
        sender.dht_find_node(Int256([9; 32]), 8, Duration::from_secs(2)),
    )
    .await
    .expect("the registered answer must leave while the inbox is full")
    .expect("the registered answer must reach the query's waiter");
    assert!(
        answer.nodes.is_empty(),
        "the registered answer's empty node list must round-trip, got {:?}",
        answer.nodes
    );
    assert!(
        receiver.inbox_drops() >= 1,
        "the consumer-bound packet behind the parked one must be counted as dropped, got {}",
        receiver.inbox_drops()
    );
}
