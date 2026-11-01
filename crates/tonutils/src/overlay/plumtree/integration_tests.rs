//! Loopback integration coverage for Plumtree over UDP and QUIC.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use tokio::sync::{RwLock, broadcast};

use crate::{
    adnl::{AdnlKeyPair, AdnlUdpSession, QuicServer, QuicSession},
    overlay::{
        AdnlUdpOverlaySession, DiscoveryStats, OverlayClient, OverlayClientInit, OverlayConfig,
        OverlayId, OverlayMemberEndpoints, OverlaySession, PeerId, PeerManager, PeerQualityConfig,
        PeerQualityRegistry, PlumtreeBroadcastId, QuicOverlaySession,
    },
};

use super::{PlumtreeConfig, PlumtreeHandle, wire};

#[derive(Clone, Copy, Debug)]
enum Transport {
    Udp,
    Quic,
}

struct TestNode {
    client: OverlayClient,
    handle: PlumtreeHandle,
}

fn config() -> PlumtreeConfig {
    PlumtreeConfig {
        k: 4,
        parts: 6,
        tree_slots: 7,
        validator_eager_limit: 2,
        eager_limit: 2,
        active_neighbours: 2,
        repair_timeout: Duration::from_millis(50),
        max_repair_targets: 2,
        enabled: true,
    }
}

fn keypair() -> AdnlKeyPair {
    AdnlKeyPair::generate(&mut rand::rngs::OsRng)
}

fn peer(keypair: &AdnlKeyPair) -> PeerId {
    PeerId::from_bytes(keypair.public_key.to_bytes())
}

async fn free_udp_addr() -> SocketAddr {
    tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
}

async fn handle(overlay: OverlayId, keypair: &AdnlKeyPair) -> PlumtreeHandle {
    handle_with_config(overlay, keypair, config()).await
}

async fn handle_with_config(
    overlay: OverlayId,
    keypair: &AdnlKeyPair,
    config: PlumtreeConfig,
) -> PlumtreeHandle {
    let handle = PlumtreeHandle::new(config);
    handle
        .configure(config, overlay, Some(keypair))
        .await
        .unwrap();
    handle
}

async fn udp_link(
    overlay: OverlayId,
    left_keypair: &AdnlKeyPair,
    left_addr: SocketAddr,
    left_handle: PlumtreeHandle,
    right_keypair: &AdnlKeyPair,
    right_addr: SocketAddr,
    right_handle: PlumtreeHandle,
) -> (Box<dyn OverlaySession>, Box<dyn OverlaySession>) {
    let left = AdnlUdpSession::connect(
        left_addr,
        right_addr,
        *left_keypair,
        right_keypair.public_key,
    )
    .await
    .unwrap();
    let right = AdnlUdpSession::connect(
        right_addr,
        left_addr,
        *right_keypair,
        left_keypair.public_key,
    )
    .await
    .unwrap();
    (
        Box::new(AdnlUdpOverlaySession::test_with_plumtree(
            left,
            peer(right_keypair),
            overlay,
            left_handle,
        )),
        Box::new(AdnlUdpOverlaySession::test_with_plumtree(
            right,
            peer(left_keypair),
            overlay,
            right_handle,
        )),
    )
}

async fn quic_link(
    overlay: OverlayId,
    client_keypair: &AdnlKeyPair,
    client_addr: SocketAddr,
    client_handle: PlumtreeHandle,
    server_keypair: &AdnlKeyPair,
    server: &QuicServer,
    server_handle: PlumtreeHandle,
) -> (Box<dyn OverlaySession>, Box<dyn OverlaySession>) {
    let server_addr = server.local_addr().unwrap();
    let client_keypair = *client_keypair;
    let client_public = client_keypair.public_key;
    let server_public = server_keypair.public_key;
    let connect = tokio::spawn(async move {
        QuicSession::connect(client_addr, server_addr, client_keypair, server_public)
            .await
            .unwrap()
    });
    let (connection, _) = server.accept().await.unwrap();
    let client = connect.await.unwrap();
    let server_session = QuicSession::from_accepted(connection, client_public);
    (
        Box::new(QuicOverlaySession::test_with_plumtree(
            Arc::new(client),
            overlay,
            client_handle,
        )),
        Box::new(QuicOverlaySession::test_with_plumtree(
            Arc::new(server_session),
            overlay,
            server_handle,
        )),
    )
}

async fn build_node(
    overlay: OverlayId,
    handle: PlumtreeHandle,
    sessions: Vec<Box<dyn OverlaySession>>,
) -> TestNode {
    let manager = PeerManager::with_overlay(OverlayConfig::default(), overlay).unwrap();
    for session in sessions {
        manager.add_session(session).await;
    }
    handle.update_peers(manager.peers().await).await;
    let client = OverlayClient::new(OverlayClientInit {
        manager,
        broadcast: None,
        quality: Arc::new(PeerQualityRegistry::new(PeerQualityConfig::default())),
        member_endpoints: OverlayMemberEndpoints::default(),
        known_peers: Arc::new(RwLock::new(Vec::new())),
        discovery: DiscoveryStats::default(),
        packet_capacity: 256,
        plumtree: Some(handle.clone()),
    });
    TestNode { client, handle }
}

async fn receive_payload(
    receiver: &mut broadcast::Receiver<crate::overlay::OverlayPacket>,
    expected: &[u8],
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let packet = receiver.recv().await.unwrap();
            if packet.payload.as_ref() == expected {
                return;
            }
        }
    })
    .await
    .expect("the expected application payload must be published");
}

async fn assert_no_payload(receiver: &mut broadcast::Receiver<crate::overlay::OverlayPacket>) {
    let duplicate = tokio::time::timeout(Duration::from_millis(500), receiver.recv()).await;
    assert!(
        duplicate.is_err(),
        "control traffic and duplicate copies must not reach subscribe_packets"
    );
}

async fn wait_for_payload(
    receiver: &mut broadcast::Receiver<crate::overlay::OverlayPacket>,
    expected: &[u8],
) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let packet = receiver.recv().await.unwrap();
            if packet.payload.as_ref() == expected {
                return;
            }
        }
    })
    .await
    .is_ok()
}

async fn run_relay(transport: Transport) {
    let overlay = OverlayId::from_name(format!("plumtree-relay-{transport:?}").as_bytes());
    let node_a = keypair();
    let node_b = keypair();
    let node_c = keypair();
    let handle_a = handle(overlay, &node_a).await;
    let handle_b = handle(overlay, &node_b).await;
    let handle_c = handle(overlay, &node_c).await;

    let (a_to_b, b_from_a);
    let (b_to_c, c_from_b);
    let _quic_server;
    match transport {
        Transport::Udp => {
            let addr_a = free_udp_addr().await;
            let addr_b1 = free_udp_addr().await;
            let addr_b2 = free_udp_addr().await;
            let addr_c = free_udp_addr().await;
            (a_to_b, b_from_a) = udp_link(
                overlay,
                &node_a,
                addr_a,
                handle_a.clone(),
                &node_b,
                addr_b1,
                handle_b.clone(),
            )
            .await;
            (b_to_c, c_from_b) = udp_link(
                overlay,
                &node_b,
                addr_b2,
                handle_b.clone(),
                &node_c,
                addr_c,
                handle_c.clone(),
            )
            .await;
            _quic_server = None;
        }
        Transport::Quic => {
            let addr_b = free_udp_addr().await;
            let server = QuicServer::bind(addr_b, node_b).await.unwrap();
            let (client_a, server_a) = quic_link(
                overlay,
                &node_a,
                free_udp_addr().await,
                handle_a.clone(),
                &node_b,
                &server,
                handle_b.clone(),
            )
            .await;
            let (client_c, server_c) = quic_link(
                overlay,
                &node_c,
                free_udp_addr().await,
                handle_c.clone(),
                &node_b,
                &server,
                handle_b.clone(),
            )
            .await;
            a_to_b = client_a;
            b_from_a = server_a;
            b_to_c = server_c;
            c_from_b = client_c;
            _quic_server = Some(server);
        }
    }

    let node_a = build_node(overlay, handle_a, vec![a_to_b]).await;
    let node_b = build_node(overlay, handle_b, vec![b_from_a, b_to_c]).await;
    let node_c = build_node(overlay, handle_c, vec![c_from_b]).await;
    let mut received = node_c.client.subscribe_packets();

    let payload = b"relayed through one eager Plumtree peer".to_vec();
    let id = PlumtreeBroadcastId::new([0x51; 32]);
    assert_eq!(
        node_a
            .client
            .broadcast_plumtree_simple(id, Arc::from(payload.clone()), 0)
            .await
            .unwrap(),
        1,
        "the origin has exactly one eager peer"
    );
    receive_payload(&mut received, &payload).await;
    assert_no_payload(&mut received).await;
    assert!(node_a.handle.stats().parts_sent >= 1);
    assert!(node_b.handle.stats().parts_received >= 1);
    assert!(node_c.handle.stats().parts_received >= 1);

    node_a.client.shutdown_wait().await;
    node_b.client.shutdown_wait().await;
    node_c.client.shutdown_wait().await;
}

#[tokio::test]
async fn udp_loopback_relays_simple_broadcast_and_hides_controls() {
    run_relay(Transport::Udp).await;
}

#[tokio::test]
async fn quic_loopback_relays_simple_broadcast_and_hides_controls() {
    run_relay(Transport::Quic).await;
}

async fn fec_repair_transport(
    transport: Transport,
    overlay: OverlayId,
    node_a: &AdnlKeyPair,
    handle_a: PlumtreeHandle,
    node_b: &AdnlKeyPair,
    handle_b: PlumtreeHandle,
) -> (PeerManager, Box<dyn OverlaySession>, Option<QuicServer>) {
    match transport {
        Transport::Udp => {
            let (a_to_b, b_from_a) = udp_link(
                overlay,
                node_a,
                free_udp_addr().await,
                handle_a,
                node_b,
                free_udp_addr().await,
                handle_b,
            )
            .await;
            let manager = PeerManager::with_overlay(OverlayConfig::default(), overlay).unwrap();
            manager.add_session(a_to_b).await;
            (manager, b_from_a, None)
        }
        Transport::Quic => {
            let server = QuicServer::bind(free_udp_addr().await, *node_b)
                .await
                .unwrap();
            let (client_a, server_b) = quic_link(
                overlay,
                node_a,
                free_udp_addr().await,
                handle_a,
                node_b,
                &server,
                handle_b,
            )
            .await;
            let manager = PeerManager::with_overlay(OverlayConfig::default(), overlay).unwrap();
            manager.add_session(client_a).await;
            (manager, server_b, Some(server))
        }
    }
}

async fn run_fec_repair(transport: Transport) {
    let overlay = OverlayId::from_name(format!("plumtree-repair-{transport:?}").as_bytes());
    let node_a = keypair();
    let node_b = keypair();
    let handle_a = handle(overlay, &node_a).await;
    let handle_b = handle(overlay, &node_b).await;

    let peer_b = peer(&node_b);
    let (a_manager, b_session, _quic_server) = fec_repair_transport(
        transport,
        overlay,
        &node_a,
        handle_a.clone(),
        &node_b,
        handle_b.clone(),
    )
    .await;

    let absent_peer = PeerId::from_bytes([0xA1; 32]);
    handle_a.update_peers(vec![absent_peer]).await;

    let node_b = build_node(overlay, handle_b, vec![b_session]).await;
    let mut received = node_b.client.subscribe_packets();

    let payload = (0..300u32)
        .map(|value| u8::try_from(value % 251).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        handle_a.broadcast_fec(payload.clone(), 0).await.unwrap(),
        config().parts as usize
    );

    let encoding = wire::encode_fec(&payload, 0, &config()).unwrap();
    let timestamp = wire::unix_seconds();
    let mut parts = Vec::new();
    for (part_index, symbol) in encoding.symbols.iter().enumerate() {
        let part_index = u32::try_from(part_index).unwrap();
        let (wire, part) = wire::fec_signed_part(
            &node_a,
            wire::FecPartSpec {
                flags: 0,
                timestamp,
                id: encoding.broadcast_id.clone(),
                full_data_hash: encoding.full_data_hash.clone(),
                full_data_size: i32::try_from(payload.len()).unwrap(),
                part_index,
            },
            symbol,
        )
        .unwrap();
        parts.push((wire, part));
    }

    let missing_index = encoding.source_count.saturating_sub(1);
    for (wire, _) in parts.iter().take(missing_index as usize) {
        assert!(
            a_manager
                .send_object_to(peer_b, Arc::from(wire.clone()))
                .await
        );
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), received.recv())
            .await
            .is_err(),
        "a lost source symbol must prevent application delivery"
    );

    let tree_index = missing_index + wire::PLUMTREE_FEC_TREE_OFFSET;
    let ihave = handle_a
        .advertise_for_test((encoding.broadcast_id.0, missing_index, tree_index), peer_b)
        .await
        .expect("the withheld part must be advertised before it can be repaired");
    handle_a.update_peers(vec![absent_peer, peer_b]).await;
    assert!(a_manager.send_object_to(peer_b, Arc::from(ihave)).await);
    assert!(
        wait_for_payload(&mut received, &payload).await,
        "repair payload was not published: origin={:?} receiver={:?}",
        handle_a.stats(),
        node_b.handle.stats()
    );
    assert_no_payload(&mut received).await;
    assert!(handle_a.stats().repair_requests >= 1);

    node_b.client.shutdown_wait().await;
    a_manager.shutdown_wait().await;
}

#[tokio::test]
async fn udp_loopback_repairs_a_lost_fec_part_once() {
    run_fec_repair(Transport::Udp).await;
}

#[tokio::test]
async fn quic_loopback_repairs_a_lost_fec_part_once() {
    run_fec_repair(Transport::Quic).await;
}

#[cfg(feature = "overlay-config")]
#[tokio::test]
#[ignore = "requires public mainnet overlay access"]
async fn mainnet_public_overlay_receives_a_completed_plumtree_payload() {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.unwrap();
    let local_addr = socket.local_addr().unwrap();
    drop(socket);
    let client = crate::overlay::OverlayNodeBuilder::new()
        .native_udp(local_addr, keypair(), None)
        .bootstrap_timeout(Duration::from_secs(30))
        .discovery_timeout(Duration::from_secs(10))
        .start_client()
        .await
        .expect("the public mainnet overlay must bootstrap");
    let mut packets = client.subscribe_packets();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let stats = client.plumtree_stats();
        if stats.parts_received > 0 && stats.completed_broadcasts > 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no completed Plumtree payload within 60 seconds: peers={} stats={stats:?}",
            client.peer_count().await,
        );
        match tokio::time::timeout(Duration::from_millis(250), packets.recv()).await {
            Ok(Ok(packet)) => log::debug!(
                "mainnet Plumtree application payload: bytes={}",
                packet.payload.len()
            ),
            Ok(Err(error)) => panic!("overlay packet subscription closed: {error}"),
            Err(_) => {}
        }
    }
    client.shutdown_wait().await;
}
