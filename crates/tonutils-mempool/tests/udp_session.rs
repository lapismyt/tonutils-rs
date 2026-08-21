use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tonutils_adnl::{AdnlUdpSession, KeyPair};
use tonutils_mempool::{AdnlUdpOverlaySession, MempoolConfig, MempoolEvent, MempoolScanner};
use tonutils_overlay::{OverlayConfig, OverlayId, PeerId};
use tonutils_tl::Message;
use tonutils_tl::tl::network::{OverlayBroadcast, PacketContents};

#[tokio::test]
async fn udp_adnl_session_delivers_custom_payload_to_mempool_stream() {
    let sender_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let receiver_key = KeyPair::generate(&mut rand::rngs::OsRng);
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
    let mut sender = AdnlUdpSession::connect(
        sender_addr,
        receiver_addr,
        sender_key,
        receiver_key.public_key,
    )
    .await
    .unwrap();
    let adapter = AdnlUdpOverlaySession::connect(
        PeerId::from_bytes([3; 32]),
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
    let manager = tonutils_overlay::PeerManager::with_overlay(
        OverlayConfig::default(),
        OverlayId::from_name(b"mempool"),
    )
    .unwrap();
    manager.add_session(Box::new(adapter)).await;
    let mut events = Box::pin(scanner.events());
    let _receiver = scanner.spawn_overlay_receiver(manager.pool());
    sender
        .send_contents(PacketContents {
            rand1: vec![1; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(Message::Custom {
                data: tl_proto::serialize(OverlayBroadcast::Unicast {
                    data: vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3],
                }),
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
