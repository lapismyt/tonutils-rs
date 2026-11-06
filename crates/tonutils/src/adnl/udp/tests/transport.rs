//! Shared-socket transport policy: probe addressing and demux isolation.
//!
//! Both tests guard what a shared `AdnlUdpTransport` must not do: a
//! dial must not repoint an established session, and one saturated
//! session must not stall the socket's other sessions.

use std::time::Duration;

use crate::adnl::{
    AdnlKeyPair, AdnlUdpSession, AdnlUdpTransport,
    tl::{AdnlMessage, PacketContents},
};

#[cfg(feature = "overlay")]
use crate::tl::TlInt256 as Int256;

/// One peer advertises several endpoints; a probe of one of them must
/// not repoint the session at it.
#[cfg(feature = "overlay")]
#[tokio::test]
async fn probe_sessions_keep_the_adopted_address() {
    let transport = AdnlUdpTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        AdnlKeyPair::generate(&mut rand::rngs::OsRng),
    )
    .await
    .expect("transport must bind");
    let peer = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
    let local_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::bind("127.0.0.1:0".parse().unwrap(), local_key)
        .await
        .expect("transport must bind");
    let local_addr = transport.local_addr().unwrap();

    // Peer A: a real session that is never drained, so its pipeline
    // fills up: 64 datagrams in its queue and 64 in its inbox park its
    // receive loop and permanently saturate the shared demux.
    let key_a = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
    let key_b = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
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
