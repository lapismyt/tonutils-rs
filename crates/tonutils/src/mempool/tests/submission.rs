use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{
    mempool::{MempoolConfig, MempoolError},
    overlay::{
        OverlayPacket,
        tl::{OverlayBroadcast, TonNodeExternalMessageBroadcast},
    },
};

use super::{
    external_boc, external_wrapper, new_server, next_external_event, pool, routing, shutdown,
};

#[tokio::test]
async fn receives_external_messages_from_the_overlay_client() {
    let (server, _client, overlay) = new_server(MempoolConfig::default()).await;
    let mut events = Box::pin(server.events());
    let mut packets = overlay.client.subscribe_packets();
    let raw = external_boc(0xabc);
    let wrapper = external_wrapper(&raw);
    pool(&overlay)
        .ingest(OverlayPacket {
            payload: wrapper.clone().into(),
            routing: routing(),
        })
        .await
        .unwrap();
    let packet = tokio::time::timeout(Duration::from_secs(2), packets.recv())
        .await
        .expect("the overlay packet pump did not forward the packet")
        .unwrap();
    assert_eq!(packet.payload.as_ref(), wrapper.as_slice());
    let event = next_external_event(&mut events).await;
    let crate::mempool::MempoolEvent::ExternalMessage { raw_boc, .. } = event else {
        unreachable!("next_external_event only returns external messages");
    };
    assert_eq!(raw_boc.as_ref(), raw.as_slice());
    let metrics = server.metrics();
    assert_eq!(metrics.overlay_packets, 1);
    assert_eq!(metrics.accepted, 1);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn submits_external_messages_through_the_overlay_client() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    let raw = external_boc(0x1234_5678);
    let expected_hash: [u8; 32] = Sha256::digest(&raw).into();
    assert_eq!(
        client.send_external(raw.clone()).await.unwrap(),
        expected_hash
    );

    let sent = overlay.sent.lock().await.clone();
    assert_eq!(sent.len(), 1, "one signed broadcast reaches the peer");
    let broadcast = tl_proto::deserialize::<OverlayBroadcast>(&sent[0])
        .expect("the overlay object must be a signed broadcast");
    let payload = broadcast
        .payload_if_valid(crate::adnl::now_i32())
        .expect("the broadcast signature must be valid");
    let wrapper = tl_proto::deserialize::<TonNodeExternalMessageBroadcast>(payload)
        .expect("mempool must wrap the BoC in externalMessageBroadcast");
    assert_eq!(wrapper.message.data, raw);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn local_send_does_not_block_on_a_full_event_queue() {
    let (server, client, overlay) = new_server(MempoolConfig {
        event_queue_capacity: 1,
        ..MempoolConfig::default()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let raw = external_boc(9);
    let expected_hash: [u8; 32] = Sha256::digest(&raw).into();
    let hash = tokio::time::timeout(Duration::from_secs(2), client.send_external(raw))
        .await
        .expect("a local submission must not wait for event-queue capacity")
        .unwrap();
    assert_eq!(hash, expected_hash);
    assert_eq!(server.metrics().accepted, 1);
    assert_eq!(overlay.sent.lock().await.len(), 1);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn reports_broadcast_failure_without_reaccepting_the_message() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    overlay.manager.shutdown_wait().await;
    let raw = external_boc(10);
    assert!(matches!(
        client.send_external(raw.clone()).await,
        Err(MempoolError::Broadcast(_))
    ));
    assert_eq!(server.metrics().broadcast_failures, 1);
    assert_eq!(server.metrics().accepted, 1);
    assert!(client.ingest(raw, routing()).await.unwrap().is_none());
    server.shutdown_wait().await;
}

#[tokio::test]
async fn ignores_non_external_overlay_payloads_without_stopping_ingest() {
    let (server, _client, overlay) = new_server(MempoolConfig::default()).await;
    let mut events = Box::pin(server.events());
    let mut packets = overlay.client.subscribe_packets();
    pool(&overlay)
        .ingest(OverlayPacket {
            payload: [1, 2, 3, 4].as_slice().into(),
            routing: routing(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), packets.recv())
        .await
        .expect("the overlay packet pump did not forward the invalid packet")
        .unwrap();
    let raw = external_boc(11);
    pool(&overlay)
        .ingest(OverlayPacket {
            payload: external_wrapper(&raw).into(),
            routing: routing(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), packets.recv())
        .await
        .expect("the overlay packet pump did not forward the valid packet")
        .unwrap();

    let event = next_external_event(&mut events).await;
    let crate::mempool::MempoolEvent::ExternalMessage { raw_boc, .. } = event else {
        unreachable!("next_external_event only returns external messages");
    };
    assert_eq!(raw_boc.as_ref(), raw.as_slice());
    assert_eq!(server.metrics().invalid_warnings, 1);

    shutdown(server, overlay).await;
}
