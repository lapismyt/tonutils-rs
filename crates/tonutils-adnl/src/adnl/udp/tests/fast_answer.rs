//! Synchronous answers sent by a session's receive task.
//!
//! These pin the two halves of
//! [`AdnlUdpSession::register_query_answer`]: an answered query is served
//! by the receive task itself and never reaches the consumer, and a
//! registration that declines a query leaves it to the consumer
//! untouched.

use std::time::Duration;

use tonutils_tl::tl::network::DhtMessage;

use crate::AdnlUdpTransport;

use super::*;

/// Two shared sessions, one on each side, routed to each other.
async fn shared_pair() -> (AdnlUdpSession, AdnlUdpSession) {
    let sender_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let receiver_key = KeyPair::generate(&mut rand::rngs::OsRng);
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
