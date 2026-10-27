#![expect(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Test fixtures intentionally narrow generated values to protocol widths"
)]
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    adnl::{
        AdnlAddress, AdnlKeyPair, AdnlUdpSession, now_i32,
        tl::{AdnlIdShort, AdnlMessage, AdnlTlPublicKey as TlPublicKey, PacketContents},
    },
    overlay::{
        AdnlUdpOverlaySession, FEC_SYMBOL_SIZE, OverlayMemberEndpoints,
        broadcast::{fec_broadcast_parts, signed_simple_broadcast},
        inbound::OverlayMemberCache,
        packet_dedup::PacketPreDedup,
        protocol_stats,
        udp_session::valid_overlay_node,
    },
    overlay::{
        OverlayId, OverlaySession, PeerId,
        tl::{FecType, OverlayBroadcastFec, OverlayCertificate, OverlayNode, OverlayNodeToSign},
    },
    tl::TlInt256 as Int256,
};

use raptorq::Encoder;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn reassembles_single_source_raptorq_external_message() {
    let external = vec![0xb5, 0xee, 0x9c, 0x72, 1, 2, 3];
    let encoder = Encoder::with_defaults(&external, 128);
    let config = encoder.get_config();
    let packet = encoder
        .get_encoded_packets(0)
        .into_iter()
        .next()
        .expect("encoder must produce a source packet");
    let hash: [u8; 32] = Sha256::digest(&external).into();
    let local = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let remote = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    };
    let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
    let fec = OverlayBroadcastFec {
        src: TlPublicKey::Overlay { name: vec![1] },
        certificate: OverlayCertificate::Empty,
        data_hash: Int256(hash),
        data_size: external.len() as i32,
        flags: 0,
        data: packet.serialize(),
        seqno: 0,
        fec: FecType::RaptorQ {
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
    assert_eq!(payload, external);
}

#[tokio::test]
async fn waits_for_all_source_symbols_before_publishing_fec_payload() {
    let external = vec![7; 300];
    let encoder = Encoder::with_defaults(&external, 64);
    let config = encoder.get_config();
    let packets = encoder.get_encoded_packets(0);
    assert!(packets.len() > 1);
    let hash: [u8; 32] = Sha256::digest(&external).into();
    let local = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let remote = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    };
    let symbols_count = (external.len() as u64).div_ceil(config.symbol_size() as u64) as i32;
    for (index, packet) in packets.into_iter().enumerate() {
        let fec = OverlayBroadcastFec {
            src: TlPublicKey::Overlay { name: vec![2] },
            certificate: OverlayCertificate::Empty,
            data_hash: Int256(hash),
            data_size: external.len() as i32,
            flags: 0,
            data: packet.serialize(),
            seqno: index as i32,
            fec: FecType::RaptorQ {
                data_size: external.len() as i32,
                symbol_size: config.symbol_size() as i32,
                symbols_count,
            },
            date: 0,
            signature: Vec::new(),
        };
        let result = adapter.unwrap_overlay_payload(&tl_proto::serialize(fec));
        if index + 1 == symbols_count as usize {
            assert_eq!(result.unwrap(), external);
        } else {
            assert!(result.is_err());
        }
    }
}

#[tokio::test]
async fn rejects_truncated_serialized_fec_packet() {
    let local = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let remote = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    };
    let payload = tl_proto::serialize(OverlayBroadcastFec {
        src: TlPublicKey::Overlay { name: vec![3] },
        certificate: OverlayCertificate::Empty,
        data_hash: Int256([4; 32]),
        data_size: 4,
        flags: 0,
        data: vec![1, 2, 3],
        seqno: 0,
        fec: FecType::RaptorQ {
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
    let client_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
    valid_overlay.extend(signed_simple_broadcast(
        &server_key,
        vec![9, 8, 7],
        0,
        now_i32(),
    ));
    sender
        .send_contents(packet(valid_overlay.clone()))
        .await
        .unwrap();

    // UDP is lossy: a loaded CI runner can drop the loopback datagram or
    // delay its processing past any single short deadline.  The assertion is
    // that the wrong-overlay packet did not kill the session, so retransmit
    // the valid packet instead of betting on one datagram.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let payload = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !remaining.is_zero(),
            "session stopped delivering valid overlay packets"
        );
        match tokio::time::timeout(
            remaining.min(Duration::from_millis(250)),
            receiver.receive(),
        )
        .await
        {
            Ok(result) => break result.unwrap(),
            Err(_) => {
                sender
                    .send_contents(packet(valid_overlay.clone()))
                    .await
                    .unwrap();
            }
        }
    };
    assert_eq!(payload.as_ref(), [9, 8, 7]);
}

#[test]
fn validates_overlay_node_signature_and_timestamp_window() {
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let overlay = OverlayId::from_name(b"overlay");
    let now = 1_000;
    let version = now + 60;
    let public_key = TlPublicKey::Ed25519 {
        key: Int256(key.public_key.to_bytes()),
    };
    let adnl_id = AdnlAddress::from(&key.public_key).to_bytes();
    let signature = key.sign_raw(&tl_proto::serialize(OverlayNodeToSign {
        id: AdnlIdShort {
            id: Int256(adnl_id),
        },
        overlay: Int256(overlay.as_bytes()),
        version,
    }));
    let node = OverlayNode {
        id: public_key,
        overlay: Int256(overlay.as_bytes()),
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
    wrong_overlay.overlay = Int256(OverlayId::from_name(b"other").as_bytes());
    assert!(
        !valid_overlay_node(&wrong_overlay, overlay, now),
        "a node for a different overlay must be rejected"
    );
}

/// Builds the loopback adapter used by the broadcast unwrap tests.
async fn broadcast_test_adapter() -> AdnlUdpOverlaySession {
    let local = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let remote = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    }
}

/// A signed simple broadcast delivers its application payload unchanged.
#[tokio::test]
async fn unwraps_application_payload_from_signed_simple_broadcast() {
    let mut adapter = broadcast_test_adapter().await;
    let payload = vec![0xb5, 0xee, 0x9c, 0x72, 4, 4, 4];
    let payload = signed_simple_broadcast(
        &AdnlKeyPair::generate(&mut rand::rngs::OsRng),
        payload.clone(),
        0,
        now_i32(),
    );
    assert_eq!(
        adapter.unwrap_overlay_payload(&payload).unwrap(),
        vec![0xb5, 0xee, 0x9c, 0x72, 4, 4, 4],
        "the overlay layer must not interpret application payloads"
    );
}

/// A payload already formatted as a bare BoC passes through unchanged.
#[tokio::test]
async fn passes_through_signed_broadcast_payload_that_is_already_a_boc() {
    let mut adapter = broadcast_test_adapter().await;
    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 5, 5, 5];
    let payload = signed_simple_broadcast(
        &AdnlKeyPair::generate(&mut rand::rngs::OsRng),
        raw_boc.clone(),
        1,
        now_i32(),
    );
    assert_eq!(adapter.unwrap_overlay_payload(&payload).unwrap(), raw_boc);
}

/// The signed FEC parts `fec_broadcast_parts` produces - raw RaptorQ
/// symbol bytes in `data` and the symbol id in `seqno`, the wire format
/// upstream's `BroadcastsFec::process_broadcast` decodes - reassemble
/// through the session's FEC receive path back to the original
/// application payload as soon as the last source symbol lands; the trailing
/// repair symbols are optional redundancy and arrive after the
/// reassembly.
#[tokio::test]
async fn reassembles_upstream_format_fec_external_message() {
    let mut adapter = broadcast_test_adapter().await;
    let before = protocol_stats();
    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let mut raw_boc = vec![0xb5, 0xee, 0x9c, 0x72];
    raw_boc.resize(1000, 0x0a);
    let parts = fec_broadcast_parts(&keypair, &raw_boc, 0, now_i32());
    let symbols_count = raw_boc.len().div_ceil(FEC_SYMBOL_SIZE);
    assert!(symbols_count > 1, "the payload must span several symbols");
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
    // The counters are process-global and tests run in parallel, so the
    // deltas are lower bounds: every part fed to a decoder counts once,
    // split into source and repair by the wire id, and the completed
    // reassembly counts as decoded.
    let after = protocol_stats();
    assert!(
        after.fec_symbols_accepted >= before.fec_symbols_accepted + 2 * symbols_count as u64,
        "every fed part must count as an accepted symbol"
    );
    assert!(
        after.fec_repair_symbols_received
            >= before.fec_repair_symbols_received + symbols_count as u64,
        "the repair tail must count as repair symbols"
    );
    assert!(
        after.fec_decoded > before.fec_decoded,
        "the completed reassembly must count as decoded"
    );
}

/// A raw-object send reaches the peer's overlay receive path and preserves
/// the application payload - the framing `OverlayBroadcastPeer` relies on.
#[tokio::test]
async fn send_object_delivers_signed_broadcast_to_peer() {
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    };
    let mut sender = session(receiver_key.public_key.to_bytes(), sender_session);
    let mut receiver = session(sender_key.public_key.to_bytes(), receiver_session);

    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 9, 9, 9];
    let broadcast = signed_simple_broadcast(&sender_key, raw_boc.clone(), 0, now_i32());
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

/// A byte-identical copy of an already accepted broadcast is stopped by
/// the raw-payload pre-dedup before the TL parse and signature
/// verification every flood copy would otherwise pay for.
#[tokio::test]
async fn duplicate_broadcast_copies_are_suppressed_before_unwrap() {
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
        endpoints: OverlayMemberEndpoints::default(),
        pre_dedup: PacketPreDedup::default(),
    };
    let mut sender = session(receiver_key.public_key.to_bytes(), sender_session);
    let mut receiver = session(sender_key.public_key.to_bytes(), receiver_session);

    let raw_boc = vec![0xb5, 0xee, 0x9c, 0x72, 7, 7, 7];
    let broadcast: Arc<[u8]> = Arc::from(signed_simple_broadcast(
        &sender_key,
        raw_boc.clone(),
        0,
        now_i32(),
    ));
    sender.send_object(Arc::clone(&broadcast)).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(5), receiver.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.as_ref(), raw_boc.as_slice());

    // The relay-style flood copy is byte-identical, so the pre-dedup
    // must swallow it: no second delivery can surface.
    sender.send_object(broadcast).await.unwrap();
    let duplicate = tokio::time::timeout(Duration::from_secs(2), receiver.receive()).await;
    assert!(
        duplicate.is_err(),
        "a byte-identical copy must not surface as a second delivery"
    );
}
