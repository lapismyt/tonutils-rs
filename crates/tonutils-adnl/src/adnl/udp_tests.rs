use std::time::Duration;

use tl_proto::TlRead;
use tokio_util::bytes::Bytes;
use tonutils_tl::tl::network::{DhtNodesBoxed, OverlayNodesBoxed, OverlayQuery, PacketContents};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::{
    AdnlAesParams, AdnlChannelCipher, AdnlChannelPacket, AdnlError, AdnlUdpPeer, AdnlUdpSession,
    KeyPair, decrypt_direct, encrypt_direct, ordered_channel_ciphers,
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
    assert!(result.unwrap().nodes.is_empty());
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
    assert!(matches!(
        server_result.unwrap().message,
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
        let AdnlMessage::Query { query_id, query } = packet.message.unwrap() else {
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
        assert!(query.is_empty());
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
    assert!(result.unwrap().nodes.is_empty());
}
