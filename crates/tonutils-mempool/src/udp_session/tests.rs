use super::*;
use crate::broadcast::{external_message_wrapper, fec_broadcast_parts, signed_simple_broadcast};
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
        endpoints: crate::OverlayMemberEndpoints::default(),
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
        endpoints: crate::OverlayMemberEndpoints::default(),
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
        endpoints: crate::OverlayMemberEndpoints::default(),
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
    let sender =
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

/// Builds the loopback adapter used by the broadcast unwrap tests.
async fn broadcast_test_adapter() -> AdnlUdpOverlaySession {
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
    AdnlUdpOverlaySession {
        peer: PeerId::from_bytes([9; 32]),
        session,
        overlay: None,
        fec: HashMap::new(),
        last_keepalive: Instant::now(),
        last_activity: Instant::now(),
        members: OverlayMemberCache::default(),
        endpoints: crate::OverlayMemberEndpoints::default(),
    }
}

/// A signed simple broadcast delivers the raw BoC nested inside the
/// `tonNode.externalMessageBroadcast` wrapper, which is the only shape
/// upstream and pytoniq broadcast externals in.
#[tokio::test]
async fn unwraps_external_message_broadcast_from_signed_simple_broadcast() {
    let mut adapter = broadcast_test_adapter().await;
    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 4, 4, 4];
    let payload = signed_simple_broadcast(
        &KeyPair::generate(&mut rand::rngs::OsRng),
        external_message_wrapper(&raw_boc),
        0,
        now_i32(),
    );
    assert_eq!(
        adapter.unwrap_overlay_payload(&payload).unwrap(),
        raw_boc,
        "the wrapper's message.data must reach the scanner as a bare BoC"
    );
}

/// A broadcast whose payload is already a bare BoC passes through unchanged,
/// so peers that skip the wrapper keep working.
#[tokio::test]
async fn passes_through_signed_broadcast_payload_that_is_already_a_boc() {
    let mut adapter = broadcast_test_adapter().await;
    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 5, 5, 5];
    let payload = signed_simple_broadcast(
        &KeyPair::generate(&mut rand::rngs::OsRng),
        raw_boc.clone(),
        1,
        now_i32(),
    );
    assert_eq!(adapter.unwrap_overlay_payload(&payload).unwrap(), raw_boc);
}

/// The signed FEC parts `fec_broadcast_parts` produces - raw RaptorQ
/// symbol bytes in `data` and the symbol id in `seqno`, the wire format
/// upstream's `BroadcastsFec::process_broadcast` decodes - reassemble
/// through the session's FEC receive path back to the bare external
/// message BoC as soon as the last source symbol lands; the trailing
/// repair symbols are optional redundancy and arrive after the
/// reassembly.
#[tokio::test]
async fn reassembles_upstream_format_fec_external_message() {
    let mut adapter = broadcast_test_adapter().await;
    let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
    let mut raw_boc = vec![0xb5, 0xee, 0x9c, 0x72];
    raw_boc.resize(1000, 0x0a);
    let wrapper = external_message_wrapper(&raw_boc);
    let parts = fec_broadcast_parts(&keypair, &wrapper, 0, now_i32());
    let symbols_count = wrapper.len().div_ceil(crate::broadcast::FEC_SYMBOL_SIZE);
    assert!(symbols_count > 1, "the wrapper must span several symbols");
    assert_eq!(
        parts.len(),
        2 * symbols_count,
        "source symbols plus one repair symbol each"
    );
    for part in parts {
        let payload = adapter.unwrap_overlay_payload(&part.object);
        if part.seqno + 1 == symbols_count as i32 {
            assert_eq!(
                payload.unwrap(),
                raw_boc,
                "the last source symbol must complete the reassembly"
            );
        } else if part.seqno < symbols_count as i32 {
            assert!(
                payload.is_err(),
                "an incomplete FEC payload must not unwrap"
            );
        } else if let Ok(extra) = payload {
            // Trailing repair symbols may assemble a second time from
            // the repair set alone - that is the redundancy they
            // exist for - but only ever to the same object.
            assert_eq!(
                extra, raw_boc,
                "a repair reassembly must yield the same object"
            );
        }
    }
}

/// A raw-object send reaches the peer's overlay receive path and unwraps
/// to the bare external message BoC - the framing `OverlayBroadcastPeer`
/// relies on for mempool publication.
#[tokio::test]
async fn send_object_delivers_signed_broadcast_to_peer() {
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
    let overlay = OverlayId::from_name(b"loopback-overlay");
    let sender_session = AdnlUdpSession::connect(
        sender_addr,
        receiver_addr,
        sender_key,
        receiver_key.public_key,
    )
    .await
    .unwrap();
    let receiver_session = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    let session = |peer_key, session| AdnlUdpOverlaySession {
        peer: PeerId::from_bytes(peer_key),
        session,
        overlay: Some(overlay),
        fec: HashMap::new(),
        last_keepalive: Instant::now(),
        last_activity: Instant::now(),
        members: OverlayMemberCache::default(),
        endpoints: crate::OverlayMemberEndpoints::default(),
    };
    let mut sender = session(receiver_key.public_key.to_bytes(), sender_session);
    let mut receiver = session(sender_key.public_key.to_bytes(), receiver_session);

    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 9, 9, 9];
    let broadcast = signed_simple_broadcast(
        &sender_key,
        external_message_wrapper(&raw_boc),
        0,
        now_i32(),
    );
    sender.send_object(Arc::from(broadcast)).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(5), receiver.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        delivered.as_ref(),
        raw_boc.as_slice(),
        "the peer must receive the bare external message BoC"
    );
}
