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

    let receiver = AdnlUdpSession::connect(
        receiver_addr,
        sender_addr,
        receiver_key,
        sender_key.public_key,
    )
    .await
    .unwrap();
    {
        let sender = AdnlUdpSession::connect(
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
    let sender = AdnlUdpSession::connect(
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
    let client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();
    let server =
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
    let client =
        AdnlUdpSession::connect(client_addr, server_addr, client_key, server_key.public_key)
            .await
            .unwrap();
    let server =
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
async fn receive_address_version(receiver: &AdnlUdpSession) -> i32 {
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
    let (sender, receiver) = direct_pair().await;

    sender.send_contents(custom_packet(vec![1])).await.unwrap();
    let first = receive_address_version(&receiver).await;

    tokio::time::sleep(Duration::from_millis(1100)).await;

    sender.send_contents(custom_packet(vec![2])).await.unwrap();
    let second = receive_address_version(&receiver).await;

    assert!(
        second > first,
        "second packet must advertise a later address version, got {first} then {second}"
    );
}

/// A peer that echoes `recv_addr_list_version` holds a routable
/// `adnl.addressList` for this node (`AdnlPeerPairImpl::send_packet` writes
/// the version it keeps into every packet), and that echo - not the answered
/// query - is what lets the peer originate `overlay.ping` and broadcasts.
/// The counter tracks distinct echoing peers so a live overlay run can
/// separate "the peer cannot reach us" from "the peer does not admit us",
/// and each peer may move it only once.
#[test]
fn address_storing_counter_counts_each_peer_once() {
    let before = super::address_storing_peers();
    let peer = [0xab; 32];

    assert!(
        super::note_peer_holds_our_addr(&peer),
        "a peer's first echo must count"
    );
    assert!(
        !super::note_peer_holds_our_addr(&peer),
        "repeat echoes from the same peer must not count again"
    );
    assert_eq!(
        super::address_storing_peers(),
        before + 1,
        "the counter must move exactly once per peer"
    );
}

/// The raw-flags probe must agree with the decoder, or a live run cannot tell
/// a missing wire bit from a dropped field mapping.
#[test]
fn raw_flags_probe_reads_the_wire_word() {
    let mut contents = PacketContents {
        rand1: vec![0; 7],
        flags: (),
        from: None,
        from_short: None,
        message: None,
        messages: None,
        address: None,
        priority_address: None,
        seqno: Some(1),
        confirm_seqno: Some(1),
        recv_addr_list_version: Some(42),
        recv_priority_addr_list_version: None,
        reinit_date: None,
        dst_reinit_date: None,
        signature: None,
        rand2: vec![0; 7],
    };

    let encoded = tl_proto::serialize(contents.clone());
    let flags = super::raw_packet_flags(&encoded).expect("header must parse");
    assert_eq!(
        flags & 0x40,
        0x40,
        "f_seqno is set on every packet upstream builds, so it anchors the offset"
    );
    assert_eq!(
        flags & 0x100,
        0x100,
        "recv_addr_list_version must reach the wire when set"
    );
    assert_eq!(
        tl_proto::deserialize::<PacketContents>(&encoded)
            .expect("roundtrip")
            .recv_addr_list_version,
        Some(42),
        "decoder and probe must agree on the same bytes"
    );

    contents.recv_addr_list_version = None;
    contents.recv_priority_addr_list_version = Some(7);
    let encoded = tl_proto::serialize(contents);
    let flags = super::raw_packet_flags(&encoded).expect("header must parse");
    assert_eq!(flags & 0x100, 0, "unset field clears bit 8 on the wire");
    assert_eq!(
        flags & 0x200,
        0x200,
        "recv_priority_addr_list_version must reach the wire when set"
    );
}

#[tokio::test]
async fn raw_exchange_returns_reply_and_recovers_from_timeout() {
    use crate::AdnlUdpTransport;

    let transport = AdnlUdpTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        KeyPair::generate(&mut rand::rngs::OsRng),
    )
    .await
    .expect("transport must bind");
    let peer = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("peer must bind");

    // A first request with no reply in flight must time out, and the
    // timeout must clear the hook so the next exchange can install one.
    let quiet = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("quiet peer");
    let timeout = transport
        .raw_exchange(
            quiet.local_addr().unwrap(),
            b"ping",
            Duration::from_millis(100),
        )
        .await;
    assert!(matches!(timeout, Err(AdnlError::Timeout { .. })));

    let peer_addr = peer.local_addr().unwrap();

    // The demultiplexer drops datagrams it cannot route, so a reply only
    // reaches the caller while the exchange hook is installed.
    let serve = tokio::spawn(async move {
        let mut buf = [0u8; 64];
        let (len, sender) = peer.recv_from(&mut buf).await.expect("peer must receive");
        let mut reply = Vec::with_capacity(len + 1);
        reply.extend_from_slice(&buf[..len]);
        reply.push(0xFF);
        peer.send_to(&reply, sender).await.expect("peer must reply");
    });

    let reply = transport
        .raw_exchange(peer_addr, b"ping", Duration::from_secs(2))
        .await
        .expect("raw exchange must return the reply");
    assert_eq!(reply, b"ping\xFF");

    serve.await.expect("peer task must finish");

    // The hook is cleared once the exchange resolves, so a later request
    // goes unanswered and times out instead of reusing a stale waiter.
    let again = transport
        .raw_exchange(peer_addr, b"ping", Duration::from_millis(100))
        .await;
    assert!(matches!(again, Err(AdnlError::Timeout { .. })));
}

/// One peer advertises several endpoints; a probe of one of them must
/// not repoint the session at it.
#[tokio::test]
async fn probe_sessions_keep_the_adopted_address() {
    use crate::AdnlUdpTransport;

    let transport = AdnlUdpTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        KeyPair::generate(&mut rand::rngs::OsRng),
    )
    .await
    .expect("transport must bind");
    let peer = KeyPair::generate(&mut rand::rngs::OsRng);
    let established: std::net::SocketAddr = "127.0.0.1:41001".parse().unwrap();
    let candidate: std::net::SocketAddr = "127.0.0.1:41002".parse().unwrap();

    let session = transport.session_for_peer(peer.public_key, established);
    assert_eq!(session.remote_addr(), established);

    // A dial-only caller gets the same session and must not repoint it:
    // a concurrent sweep over candidate endpoints used to strand the
    // manager's keepalives on whichever candidate adopted last.
    let probe = transport.session_for_probe(peer.public_key, candidate);
    assert_eq!(
        probe.remote_addr(),
        established,
        "a probe must not adopt a candidate endpoint"
    );

    // An addressed send keeps the adopted address too: the destination
    // rides on that packet only.
    probe
        .send_overlay_get_random_peers_to(candidate, Int256([7; 32]))
        .await
        .expect("an addressed send must reach the socket layer");
    assert_eq!(
        session.remote_addr(),
        established,
        "an addressed send must not adopt its destination"
    );

    // The deliberate adopt path keeps working for the manager.
    transport.session_for_peer(peer.public_key, candidate);
    assert_eq!(session.remote_addr(), candidate);
}

/// A session whose consumer stalls must not block the shared socket's
/// demultiplexer.
///
/// The demultiplexer is one task for every session of a node id, and
/// its queues are bounded.  Waiting for room in a saturated session's
/// queue let one stalled consumer add head-of-line delay to every
/// other session on the socket; live CI runs showed it as a wide-area
/// inbound lag (~27 s by late run) applied uniformly to all peers.
/// Overflowing datagrams are now dropped and the demultiplexer keeps
/// serving the rest of the socket.
#[tokio::test]
async fn saturated_session_does_not_stall_the_shared_demux() {
    use crate::AdnlUdpTransport;

    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::bind("127.0.0.1:0".parse().unwrap(), local_key)
        .await
        .expect("transport must bind");
    let local_addr = transport.local_addr().unwrap();

    // Peer A: a real session that is never drained, so its pipeline
    // fills up: 64 datagrams in its queue and 64 in its inbox park its
    // receive loop and permanently saturate the shared demux.
    let key_a = KeyPair::generate(&mut rand::rngs::OsRng);
    let addr_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let _session_a = transport.session_for_peer(key_a.public_key, addr_a);
    let peer_a = AdnlUdpSession::connect(addr_a, local_addr, key_a, local_key.public_key)
        .await
        .unwrap();

    // Peer B: a real single-socket session whose signed direct packet
    // the shared transport decodes and delivers to session B.
    let key_b = KeyPair::generate(&mut rand::rngs::OsRng);
    let addr_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let session_b = transport.session_for_peer(key_b.public_key, addr_b);
    let peer_b = AdnlUdpSession::connect(addr_b, local_addr, key_b, local_key.public_key)
        .await
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

    // Overflow peer A's queue plus receive budget.
    for index in 0..200u8 {
        peer_a
            .send_contents(packet(vec![index]))
            .await
            .expect("flood must send");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Peer B's packet must survive A's backlog and arrive promptly.
    peer_b.send_contents(packet(vec![9, 9, 9])).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), session_b.recv_contents())
        .await
        .expect("a saturated sibling session must not stall session B")
        .expect("session B must decode its packet");
    assert_eq!(
        received.message,
        Some(AdnlMessage::Custom {
            data: vec![9, 9, 9]
        })
    );
}
