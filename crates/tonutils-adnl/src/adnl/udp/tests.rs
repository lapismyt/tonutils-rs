use std::time::Duration;

use tl_proto::TlRead;
use tokio_util::bytes::Bytes;
use tonutils_tl::tl::network::{DhtNodesBoxed, OverlayNodesBoxed, OverlayQuery, PacketContents};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::{
    AdnlAesParams, AdnlChannelCipher, AdnlChannelPacket, AdnlError, AdnlUdpPeer, AdnlUdpSession,
    KeyPair, decrypt_direct, encrypt_direct, now_i32, ordered_channel_ciphers,
};

#[test]
fn roundtrip_and_reject_trailing_data() {
    let params = AdnlAesParams::default();
    let address = "127.0.0.1:30303".parse().unwrap();
    let mut client = AdnlUdpPeer::client(address, &params);
    let mut server = AdnlUdpPeer::server(address, &params);
    let packet = client.encode(Bytes::from_static(&[1, 2, 3])).unwrap();
    assert_eq!(server.decode(&packet).unwrap().as_ref(), [1, 2, 3]);
    assert!(matches!(
        server.decode(&packet),
        Err(AdnlError::ReplayDetected)
    ));

    let mut malformed = packet.to_vec();
    malformed.push(0);
    assert!(server.decode(&malformed).is_err());
}

#[test]
fn channel_cipher_matches_direction_and_integrity_rules() {
    let secret = std::array::from_fn(|index| index as u8);
    let cipher = AdnlChannelCipher::new(secret);
    let encrypted = cipher.encrypt(b"channel payload");
    assert_eq!(
        cipher.decrypt(&encrypted).unwrap().as_ref(),
        b"channel payload"
    );

    let mut tampered = encrypted.to_vec();
    *tampered.last_mut().unwrap() ^= 1;
    assert!(matches!(
        cipher.decrypt(&tampered),
        Err(AdnlError::IntegrityError)
    ));

    let (outbound, inbound) = ordered_channel_ciphers([1; 32], [2; 32], secret);
    let packet = outbound.encrypt(b"ordered");
    assert!(inbound.decrypt(&packet).is_err());
    let (_, receiver_inbound) = ordered_channel_ciphers([2; 32], [1; 32], secret);
    assert_eq!(
        receiver_inbound.decrypt(&packet).unwrap().as_ref(),
        b"ordered"
    );
}

#[test]
fn channel_packet_roundtrips_and_rejects_replay() {
    let outbound = AdnlChannelCipher::new([1; 32]);
    let inbound = AdnlChannelCipher::new([2; 32]);
    let mut sender = AdnlChannelPacket::new([9; 32], outbound.clone(), inbound.clone());
    let mut receiver = AdnlChannelPacket::new([9; 32], inbound, outbound);
    let packet = sender
        .encode(PacketContents {
            rand1: vec![1],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Custom { data: vec![7, 8] }),
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
            rand2: vec![2],
        })
        .unwrap();
    let decoded = receiver.decode(&packet).unwrap();
    assert_eq!(
        decoded.message,
        Some(AdnlMessage::Custom { data: vec![7, 8] })
    );
    assert!(matches!(
        receiver.decode(&packet),
        Err(AdnlError::ReplayDetected)
    ));
}

#[test]
fn direct_packet_encryption_roundtrips_with_receiver_key() {
    let sender = KeyPair::generate(&mut rand::rngs::OsRng);
    let receiver = KeyPair::generate(&mut rand::rngs::OsRng);
    let encrypted = encrypt_direct(&receiver.public_key, b"direct packet");
    let (_, plaintext) = decrypt_direct(&receiver, &encrypted).unwrap();
    assert_eq!(plaintext.as_ref(), b"direct packet");
    assert!(decrypt_direct(&sender, &encrypted).is_err());
}

#[tokio::test]
async fn direct_session_roundtrips_signed_packet_and_rejects_replay() {
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
    let mut receiver = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    let packet = PacketContents {
        rand1: vec![1],
        flags: (),
        from: None,
        from_short: None,
        message: Some(AdnlMessage::Custom {
            data: vec![4, 5, 6],
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
        rand2: vec![2],
    };
    sender.send_contents(packet).await.unwrap();
    let received = receiver.recv_timeout(Duration::from_secs(1)).await.unwrap();
    assert_eq!(
        received.message,
        Some(AdnlMessage::Custom {
            data: vec![4, 5, 6]
        })
    );
}

/// A later session from the same node id must continue the peer pair's
/// outgoing sequence numbers instead of restarting at 1.
///
/// Upstream keeps one `AdnlPeerPairImpl` per peer pair, so a session that
/// restarts `seqno` has every packet rejected as a replay; that is exactly
/// what made discovery-phase queries unanswered once the process had already
/// contacted the same seed.  The receiver below stays alive across both
/// sender sessions, which is what the real peer does.
#[tokio::test]
async fn second_session_to_same_peer_continues_sequence_numbers() {
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

    let packet = |data: Vec<u8>| PacketContents {
        rand1: vec![1],
        flags: (),
        from: None,
        from_short: None,
        message: Some(AdnlMessage::Custom { data }),
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
        rand2: vec![2],
    };

    let mut receiver = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    {
        let mut sender = AdnlUdpSession::connect(
            sender_addr,
            receiver_addr,
            sender_key,
            receiver_key.public_key,
        )
        .await
        .unwrap();
        sender.send_contents(packet(vec![1, 2, 3])).await.unwrap();
        let received = receiver.recv_timeout(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            received.message,
            Some(AdnlMessage::Custom {
                data: vec![1, 2, 3]
            })
        );
    }

    // The peer keeps its sequence number state after our session is gone, so
    // the replacement session must pick up where the old one stopped.
    let mut sender = AdnlUdpSession::connect(
        sender_addr,
        receiver_addr,
        sender_key,
        receiver_key.public_key,
    )
    .await
    .unwrap();
    sender.send_contents(packet(vec![7, 8])).await.unwrap();
    let received = receiver
        .recv_timeout(Duration::from_secs(1))
        .await
        .expect("second session's packet was rejected as a replay");
    assert_eq!(
        received.message,
        Some(AdnlMessage::Custom { data: vec![7, 8] })
    );
}

#[tokio::test]
async fn dht_find_node_query_routes_matching_answer() {
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
    let mut receiver = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    let response = async {
        let packet = receiver.recv_timeout(Duration::from_secs(1)).await.unwrap();
        let AdnlMessage::Query { query_id, .. } = packet.message.unwrap() else {
            panic!("expected DHT query");
        };
        receiver
            .send_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: Some(AdnlMessage::Answer {
                    query_id,
                    answer: tl_proto::serialize(DhtNodesBoxed { nodes: Vec::new() }),
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
            .unwrap();
    };
    let (result, ()) = tokio::join!(
        sender.dht_find_node(Int256([8; 32]), 8, Duration::from_secs(1)),
        response
    );
    assert_eq!(
        result.unwrap().nodes,
        [] as [tonutils_tl::network::DhtNode; 0]
    );
}

#[tokio::test]
async fn channel_create_confirm_switches_to_directional_channel_packets() {
    let client_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let server_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let mut client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();
    let mut server =
        AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
            .await
            .unwrap();
    let (client_result, server_result) = tokio::join!(
        client.establish_channel(Duration::from_secs(1)),
        server.recv_timeout(Duration::from_secs(1))
    );
    client_result.unwrap();
    let server_packet = server_result.unwrap();
    assert!(server_packet.message.is_none());
    assert!(matches!(
        server_packet
            .messages
            .as_ref()
            .and_then(|messages| messages.first()),
        Some(AdnlMessage::CreateChannel { .. })
    ));
    client
        .send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Custom {
                data: vec![1, 2, 3],
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
        .unwrap();
    let received = server.recv_timeout(Duration::from_secs(1)).await.unwrap();
    assert_eq!(
        received.message,
        Some(AdnlMessage::Custom {
            data: vec![1, 2, 3]
        })
    );
}

#[tokio::test]
async fn overlay_random_peers_query_routes_boxed_response() {
    let client_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let server_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let client_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let server_addr = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let mut client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();
    let mut server =
        AdnlUdpSession::connect(server_addr, client_addr, server_key, client_key.public_key)
            .await
            .unwrap();
    let response = async {
        let packet = server.recv_timeout(Duration::from_secs(1)).await.unwrap();
        // The session bundles `adnl.message.createChannel` with its first
        // query, so the query can arrive as the packet's single `message` or
        // as the second entry of `messages`.
        let mut incoming = packet
            .message
            .into_iter()
            .chain(packet.messages.into_iter().flatten());
        let query = incoming.find_map(|message| match message {
            AdnlMessage::Query { query_id, query } => Some((query_id, query)),
            _ => None,
        });
        let Some((query_id, query)) = query else {
            panic!("expected overlay query");
        };
        assert_eq!(&query[..4], &0xccfd8443u32.to_le_bytes());
        let mut query = query.as_slice();
        assert!(matches!(
            OverlayQuery::read_from(&mut query),
            Ok(OverlayQuery::Query { .. })
        ));
        let Ok(OverlayQuery::GetRandomPeers { peers }) = OverlayQuery::read_from(&mut query) else {
            panic!("expected getRandomPeers query");
        };
        assert_eq!(peers.nodes.len(), 1);
        assert_eq!(peers.nodes[0].signature.len(), 64);
        assert_eq!(query, &[] as &[u8]);
        server
            .send_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: Some(AdnlMessage::Answer {
                    query_id,
                    answer: tl_proto::serialize(OverlayNodesBoxed { nodes: Vec::new() }),
                }),
                messages: None,
                address: None,
                priority_address: None,
                recv_addr_list_version: None,
                recv_priority_addr_list_version: None,
                seqno: None,
                confirm_seqno: None,
                reinit_date: None,
                dst_reinit_date: None,
                signature: None,
                rand2: vec![0; 7],
            })
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(
        client.overlay_get_random_peers(Int256([12; 32]), Duration::from_secs(1)),
        response
    );
    assert_eq!(
        result.unwrap().nodes,
        [] as [tonutils_tl::network::OverlayNode; 0]
    );
}

/// Two directly connected sessions over loopback, sender first.
async fn direct_pair() -> (AdnlUdpSession, AdnlUdpSession) {
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
    let sender = AdnlUdpSession::connect(
        sender_addr,
        receiver_addr,
        sender_key,
        receiver_key.public_key,
    )
    .await
    .unwrap();
    let receiver = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    (sender, receiver)
}

fn custom_packet(data: Vec<u8>) -> PacketContents {
    PacketContents {
        rand1: vec![0; 7],
        flags: (),
        from: None,
        from_short: None,
        message: Some(AdnlMessage::Custom { data }),
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
    }
}

/// Version of `adnl.addressList` the sender stamped on its next packet.
async fn receive_address_version(receiver: &mut AdnlUdpSession) -> i32 {
    let received = receiver
        .recv_timeout(Duration::from_secs(1))
        .await
        .expect("packet did not arrive");
    received
        .address
        .expect("every outgoing packet carries an address list")
        .version
}

/// Upstream learns a peer's source address only from `packet.addr_list()`
/// (`AdnlPeerPairImpl::receive_packet_checked`) and keeps one per peer pair,
/// replaced only on a strictly greater `version`.  A version frozen at connect
/// time would therefore let the first session to reach a peer own that address
/// for good, so every packet stamps the time it is sent and a live session
/// takes the address back on its next keepalive.
#[tokio::test]
async fn address_version_is_restamped_on_every_packet() {
    let (mut sender, mut receiver) = direct_pair().await;

    sender.send_contents(custom_packet(vec![1])).await.unwrap();
    let first = receive_address_version(&mut receiver).await;

    tokio::time::sleep(Duration::from_millis(1100)).await;

    sender.send_contents(custom_packet(vec![2])).await.unwrap();
    let second = receive_address_version(&mut receiver).await;

    assert!(
        second > first,
        "second packet must advertise a later address version, got {first} then {second}"
    );
}

/// Two sessions of the same ADNL node id that send inside the same second tie
/// on `version` and the incumbent keeps the address, so a one-shot lookup
/// socket would never receive the answer to its own query.  A session marked
/// with [`AdnlUdpSession::set_transient_address`] advertises one second ahead
/// of the wall clock and wins that single exchange; the live session's next
/// keepalive then carries a later timestamp and reclaims the address.
#[tokio::test]
async fn transient_session_stamps_one_second_ahead_of_a_plain_session() {
    let (mut plain_sender, mut plain_receiver) = direct_pair().await;
    let (mut marked_sender, mut marked_receiver) = direct_pair().await;
    marked_sender.set_transient_address(true);

    let before = now_i32();
    plain_sender
        .send_contents(custom_packet(vec![1]))
        .await
        .unwrap();
    marked_sender
        .send_contents(custom_packet(vec![2]))
        .await
        .unwrap();
    let after = now_i32();

    let plain = receive_address_version(&mut plain_receiver).await;
    let marked = receive_address_version(&mut marked_receiver).await;

    assert!(
        (before..=after).contains(&plain),
        "plain session must stamp the wall clock, got {plain} outside [{before}, {after}]"
    );
    assert!(
        (before + 1..=after + 1).contains(&marked),
        "transient session must stamp one second ahead, got {marked} outside [{before}, {after}] + 1"
    );
}
