use std::sync::Arc;
use std::time::Duration;

use crate::{
    adnl::{AdnlKeyPair, AdnlUdpSession},
    mempool::{AdnlUdpOverlaySession, MempoolConfig, MempoolEvent, MempoolScanner},
    overlay::{OverlayConfig, OverlayId, PeerId, PeerManager},
    tl::{
        AdnlMessage, Int256,
        network::{OverlayBroadcast, OverlayMessage, OverlayPong, OverlayQuery, PacketContents},
    },
};

use futures::StreamExt;
use tl_proto::TlRead;

#[tokio::test]
async fn udp_adnl_session_delivers_custom_payload_to_mempool_stream() {
    let sender_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let receiver_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let sender_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let receiver_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let sender = AdnlUdpSession::connect(
        sender_addr,
        receiver_addr,
        sender_key,
        receiver_key.public_key,
    )
    .await
    .unwrap();
    let overlay = OverlayId::MAINNET_BASECHAIN_OVERLAY_ID;
    let adapter = AdnlUdpOverlaySession::connect_for_overlay(
        PeerId::from_bytes([3; 32]),
        overlay,
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    let scanner = Arc::new(
        MempoolScanner::new(MempoolConfig {
            validate_message: false,
            ..Default::default()
        })
        .unwrap(),
    );
    let manager = PeerManager::with_overlay(
        OverlayConfig::default(),
        OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
    )
    .unwrap();
    manager.add_session(Box::new(adapter)).await;
    let mut events = Box::pin(scanner.events());
    let _receiver = scanner.spawn_overlay_receiver(manager.pool());
    let mut overlay_payload = Vec::new();
    overlay_payload.extend_from_slice(&0x75252420u32.to_le_bytes());
    overlay_payload.extend_from_slice(&overlay.as_bytes());
    overlay_payload.extend(tl_proto::serialize(OverlayBroadcast::Unicast {
        data: vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3],
    }));
    let mut envelope = overlay_payload.as_slice();
    assert!(matches!(
        OverlayMessage::read_from(&mut envelope),
        Ok(OverlayMessage::Message { .. })
    ));
    assert!(matches!(
        OverlayBroadcast::read_from(&mut envelope),
        Ok(OverlayBroadcast::Unicast { .. })
    ));
    sender
        .send_contents(PacketContents {
            rand1: vec![1; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Custom {
                data: overlay_payload,
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
            rand2: vec![2; 7],
        })
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, MempoolEvent::ExternalMessage { .. }));
    manager.shutdown();
}

#[tokio::test]
async fn adnl_session_sends_answer_to_query() {
    let client_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let server_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    drop(server_socket);

    let server =
        AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
            .await
            .unwrap();
    let client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();

    // Server sends an answer
    let query_id = Int256::random();
    server
        .send_answer(query_id.clone(), tl_proto::serialize(OverlayQuery::Ping))
        .await
        .unwrap();

    // Client receives the answer
    let packet = tokio::time::timeout(Duration::from_secs(2), client.recv_contents())
        .await
        .unwrap()
        .unwrap();

    match packet.message {
        Some(AdnlMessage::Answer {
            answer,
            query_id: id,
        }) => {
            assert_eq!(id, query_id);
            assert!(matches!(
                OverlayQuery::read_from(&mut answer.as_slice()),
                Ok(OverlayQuery::Ping)
            ));
        }
        _ => panic!("expected Answer message with Ping"),
    }
}

/// The scanner's overlay sessions must answer peer liveness probes without
/// help from the consumer: `connect_for_overlay` registers the synchronous
/// answers on the session, so its receive task sends the pong even though
/// nothing in this test ever reads the session's inbox.
///
/// Upstream drops a peer whose probe goes unanswered past its own deadline
/// (`OverlayImpl::process_pending_peer`), which is the state run 39114243866
/// recorded on every sample (`pushers=0`).
#[tokio::test]
async fn overlay_session_fast_answers_a_wrapped_ping_without_a_consumer() {
    let server_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let overlay = OverlayId::MAINNET_BASECHAIN_OVERLAY_ID;

    // The adapter is the mempool's own overlay session; its consumer never
    // runs here, which is what makes this test the gate's own shape.
    let _adapter = AdnlUdpOverlaySession::connect_for_overlay(
        PeerId::from_bytes([7; 32]),
        overlay,
        server_addr,
        client_addr,
        server_key,
        client_key.public_key,
    )
    .await
    .unwrap();
    let client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();

    let mut ping = tl_proto::serialize(OverlayQuery::Query {
        overlay: Int256(overlay.as_bytes()),
    });
    ping.extend_from_slice(&tl_proto::serialize(OverlayQuery::Ping));
    let query_id = Int256::random();
    client
        .send_contents(PacketContents {
            rand1: vec![1; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query: ping,
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
            rand2: vec![2; 7],
        })
        .await
        .unwrap();

    // The adapter's constructor sent `overlay.getRandomPeers` as well, so
    // skip whatever the peer sees before the pong.
    let expected = tl_proto::serialize(OverlayPong);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let packet = tokio::time::timeout(remaining, client.recv_contents())
            .await
            .expect("the registered answer must arrive without a consumer")
            .unwrap();
        if let Some(AdnlMessage::Answer {
            answer,
            query_id: id,
        }) = packet.message
        {
            assert_eq!(id, query_id);
            assert_eq!(answer, expected);
            break;
        }
    }
}
