use super::*;
use raptorq::Encoder;
use tonutils_adnl::KeyPair;
use tonutils_tl::tl::network::{TonNodeExternalMessage, TonNodeExternalMessageBroadcast};

#[tokio::test]
async fn reassembles_single_source_raptorq_external_message() {
    let external = tl_proto::serialize(TonNodeExternalMessageBroadcast {
        message: TonNodeExternalMessage {
            data: vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3],
        },
    });
    let encoder = Encoder::with_defaults(&external, 128);
    let config = encoder.get_config();
    let packet = encoder
        .get_encoded_packets(0)
        .into_iter()
        .next()
        .expect("encoder must produce a source packet");
    let hash: [u8; 32] = Sha256::digest(&external).into();
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
    let session = AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
        .await
        .unwrap();
    let mut adapter = AdnlUdpOverlaySession {
        peer: PeerId::from_bytes([1; 32]),
        session,
        overlay: None,
        fec: HashMap::new(),
        last_keepalive: Instant::now(),
        last_activity: Instant::now(),
        members: OverlayMemberCache::default(),
    };
    let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
    let fec = OverlayBroadcastFec {
        src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![1] },
        certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
        data_hash: tonutils_tl::Int256(hash),
        data_size: external.len() as i32,
        flags: 0,
        data: packet.serialize(),
        seqno: 0,
        fec: tonutils_tl::tl::network::FecType::RaptorQ {
            data_size: external.len() as i32,
            symbol_size: config.symbol_size() as i32,
            symbols_count,
        },
        date: 0,
        signature: Vec::new(),
    };
    let payload = adapter
        .unwrap_overlay_payload(&tl_proto::serialize(fec))
        .unwrap();
    assert_eq!(payload, vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3]);
}

#[tokio::test]
async fn waits_for_all_source_symbols_before_publishing_fec_payload() {
    let external = tl_proto::serialize(TonNodeExternalMessageBroadcast {
        message: TonNodeExternalMessage { data: vec![7; 300] },
    });
    let encoder = Encoder::with_defaults(&external, 64);
    let config = encoder.get_config();
    let packets = encoder.get_encoded_packets(0);
    assert!(packets.len() > 1);
    let hash: [u8; 32] = Sha256::digest(&external).into();
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
    let session = AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
        .await
        .unwrap();
    let mut adapter = AdnlUdpOverlaySession {
        peer: PeerId::from_bytes([2; 32]),
        session,
        overlay: None,
        fec: HashMap::new(),
        last_keepalive: Instant::now(),
        last_activity: Instant::now(),
        members: OverlayMemberCache::default(),
    };
    let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
    for (index, packet) in packets.into_iter().enumerate() {
        let fec = OverlayBroadcastFec {
            src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![2] },
            certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
            data_hash: tonutils_tl::Int256(hash),
            data_size: external.len() as i32,
            flags: 0,
            data: packet.serialize(),
            seqno: index as i32,
            fec: tonutils_tl::tl::network::FecType::RaptorQ {
                data_size: external.len() as i32,
                symbol_size: config.symbol_size() as i32,
                symbols_count,
            },
            date: 0,
            signature: Vec::new(),
        };
        let result = adapter.unwrap_overlay_payload(&tl_proto::serialize(fec));
        if index + 1 == symbols_count as usize {
            assert_eq!(result.unwrap(), vec![7; 300]);
        } else {
            assert!(result.is_err());
        }
    }
}

#[tokio::test]
async fn rejects_truncated_serialized_fec_packet() {
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
    let session = AdnlUdpSession::connect(local_addr, remote_addr, local, remote.public_key)
        .await
        .unwrap();
    let mut adapter = AdnlUdpOverlaySession {
        peer: PeerId::from_bytes([3; 32]),
        session,
        overlay: None,
        fec: HashMap::new(),
        last_keepalive: Instant::now(),
        last_activity: Instant::now(),
        members: OverlayMemberCache::default(),
    };
    let payload = tl_proto::serialize(OverlayBroadcastFec {
        src: tonutils_tl::tl::network::PublicKey::Overlay { name: vec![3] },
        certificate: tonutils_tl::tl::network::OverlayCertificate::Empty,
        data_hash: tonutils_tl::Int256([4; 32]),
        data_size: 4,
        flags: 0,
        data: vec![1, 2, 3],
        seqno: 0,
        fec: tonutils_tl::tl::network::FecType::RaptorQ {
            data_size: 4,
            symbol_size: 4,
            symbols_count: 1,
        },
        date: 0,
        signature: Vec::new(),
    });
    assert_eq!(
        adapter.unwrap_overlay_payload(&payload).unwrap_err(),
        "overlay FEC packet is truncated"
    );
}

#[tokio::test]
async fn ignores_wrong_overlay_packets_without_killing_session() {
    let client_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let server_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server_socket.local_addr().unwrap();
    let overlay = OverlayId::from_name(b"expected-overlay");
    let mut receiver = AdnlUdpOverlaySession::connect_for_overlay(
        PeerId::from_bytes(server_key.public_key.to_bytes()),
        overlay,
        client_addr,
        server_addr,
        client_key,
        server_key.public_key,
    )
    .await
    .unwrap();
    drop(server_socket);
    let mut sender =
        AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
            .await
            .unwrap();

    let mut wrong_overlay = Vec::new();
    wrong_overlay.extend_from_slice(&0x75252420u32.to_le_bytes());
    wrong_overlay.extend_from_slice(&OverlayId::from_name(b"wrong-overlay").as_bytes());
    wrong_overlay.extend_from_slice(&[1, 2, 3]);
    let packet = |data| PacketContents {
        rand1: vec![0; 7],
        flags: (),
        from: None,
        from_short: None,
        message: Some(AdnlMessage::Custom { data }),
        messages: None,
        address: None,
        priority_address: None,
        recv_addr_list_version: None,
        recv_priority_addr_list_version: None,
        reinit_date: None,
        dst_reinit_date: None,
        signature: None,
        rand2: vec![0; 7],
        seqno: None,
        confirm_seqno: None,
    };
    sender.send_contents(packet(wrong_overlay)).await.unwrap();

    let mut valid_overlay = Vec::new();
    valid_overlay.extend_from_slice(&0x75252420u32.to_le_bytes());
    valid_overlay.extend_from_slice(&overlay.as_bytes());
    valid_overlay.extend(tl_proto::serialize(TonNodeExternalMessageBroadcast {
        message: TonNodeExternalMessage {
            data: vec![9, 8, 7],
        },
    }));
    sender.send_contents(packet(valid_overlay)).await.unwrap();

    let payload = tokio::time::timeout(Duration::from_secs(1), receiver.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payload.as_ref(), [9, 8, 7]);
}

#[test]
fn validates_overlay_node_signature_and_timestamp_window() {
    let key = KeyPair::generate(&mut rand::rngs::OsRng);
    let overlay = OverlayId::from_name(b"overlay");
    let now = 1_000;
    let version = now + 60;
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
    let node = OverlayNode {
        id: public_key,
        overlay: tonutils_tl::Int256(overlay.as_bytes()),
        version,
        signature: signature.to_vec(),
    };
    assert!(valid_overlay_node(&node, overlay, now));
    let mut prefixed = vec![0xff, 0xff, 0xff, 0xff];
    prefixed.extend_from_slice(&signature);
    let prefixed_node = OverlayNode {
        signature: prefixed,
        ..node.clone()
    };
    assert!(valid_overlay_node(&prefixed_node, overlay, now));
    assert!(!valid_overlay_node(&node, overlay, now + 1_100));

    let mut corrupted = Vec::from(signature);
    corrupted[0] ^= 0xff;
    let corrupted_node = OverlayNode {
        signature: corrupted,
        ..node.clone()
    };
    assert!(
        !valid_overlay_node(&corrupted_node, overlay, now),
        "a corrupted signature must be rejected"
    );

    let mut wrong_overlay = node.clone();
    wrong_overlay.overlay = tonutils_tl::Int256(OverlayId::from_name(b"other").as_bytes());
    assert!(
        !valid_overlay_node(&wrong_overlay, overlay, now),
        "a node for a different overlay must be rejected"
    );
}
