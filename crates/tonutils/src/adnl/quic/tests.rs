use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::adnl::{
    AdnlError, AdnlKeyPair, QuicServer, QuicSession,
    quic::{
        MAX_FRAME_SIZE,
        tls::{make_quinn_client_config, make_quinn_client_config_without_identity},
    },
    sni_for_public_key,
    tl::{QuicAnswer, QuicQuery},
};

#[cfg(feature = "overlay")]
use crate::{
    overlay::tl::{OverlayNodes, OverlayNodesBoxed, OverlayQuery},
    tl::TlInt256 as Int256,
};

/// Connects a raw quinn client (caller-supplied config and SNI) to
/// `remote_addr` so negative tests can dial with a deliberate TLS
/// mismatch while keeping the transport identical to [`QuicSession::connect`].
async fn connect_raw_client(
    config: quinn::ClientConfig,
    remote_addr: SocketAddr,
    sni: &str,
) -> Result<quinn::Connection, AdnlError> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    socket.set_nonblocking(true)?;
    let endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        None,
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

    endpoint
        .connect_with(config, remote_addr, sni)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?
        .await
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))
}

#[test]
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn sni_for_public_key_produces_valid_format() {
    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let sni = sni_for_public_key(&keypair.public_key);
    assert!(sni.ends_with(".adnl"));
    let parts: Vec<&str> = sni.trim_end_matches(".adnl").split('.').collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].len(), 32);
    assert_eq!(parts[1].len(), 32);
}

#[tokio::test]
async fn quic_session_connects_and_exchanges_query() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();

    let client_handle = tokio::spawn({
        let pk = server_keypair.public_key;
        async move {
            QuicSession::connect(
                "127.0.0.1:0".parse().unwrap(),
                server_addr,
                client_keypair,
                pk,
            )
            .await
            .unwrap()
        }
    });

    let (server_conn, _sni) = server.accept().await.unwrap();
    let server_fut = async {
        let (mut send, mut recv) = server_conn.accept_bi().await.unwrap();
        let buf = recv.read_to_end(MAX_FRAME_SIZE).await.unwrap();
        let _query: QuicQuery = tl_proto::deserialize(&buf).unwrap();
        let answer = QuicAnswer { data: vec![42] };
        let answer_bytes = tl_proto::serialize(answer);
        send.write_all(&answer_bytes).await.unwrap();
        send.finish().unwrap();
        server_conn.closed().await;
    };

    let client_fut = async {
        let client = client_handle.await.unwrap();
        client
            .send_query(vec![1, 2, 3], Duration::from_secs(5))
            .await
            .unwrap()
    };

    let (_server_result, response) = tokio::join!(server_fut, client_fut);
    assert_eq!(response, vec![42]);
}

#[cfg(feature = "overlay")]
#[tokio::test]
async fn overlay_get_random_peers_wraps_the_overlay_query_header() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();

    let client_handle = tokio::spawn({
        let pk = server_keypair.public_key;
        async move {
            QuicSession::connect(
                "127.0.0.1:0".parse().unwrap(),
                server_addr,
                client_keypair,
                pk,
            )
            .await
            .unwrap()
        }
    });

    let (server_conn, _sni) = server.accept().await.unwrap();
    let server_fut = async move {
        let (mut send, mut recv) = server_conn.accept_bi().await.unwrap();
        let buf = recv.read_to_end(MAX_FRAME_SIZE).await.unwrap();
        let query: QuicQuery = tl_proto::deserialize(&buf).unwrap();

        // Exact byte equality proves all four properties at once: the
        // `overlay.query overlay:int256 = True` header (4-byte
        // constructor + 32-byte overlay id, no length prefix), the
        // overlay id value itself, the RAW unlength-prefixed suffix,
        // and the inner `overlay.getRandomPeers` constructor with an
        // empty `overlay.nodes`.
        let mut expected = tl_proto::serialize(OverlayQuery::Query {
            overlay: Int256([7; 32]),
        });
        expected.extend(tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: OverlayNodes { nodes: vec![] },
        }));
        assert_eq!(
            query.data, expected,
            "the quic.query payload must be the overlay.query header \
         (constructor + overlay id) followed by the raw \
         overlay.getRandomPeers query"
        );

        let answer = QuicAnswer {
            data: tl_proto::serialize(OverlayNodesBoxed { nodes: vec![] }),
        };
        let answer_bytes = tl_proto::serialize(answer);
        send.write_all(&answer_bytes).await.unwrap();
        send.finish().unwrap();
        server_conn.closed().await;
    };

    let client_fut = async {
        let client = client_handle.await.unwrap();
        client
            .overlay_get_random_peers(Int256([7; 32]), Duration::from_secs(5))
            .await
            .unwrap()
    };

    let ((), nodes) = tokio::join!(server_fut, client_fut);
    assert!(
        nodes.nodes.is_empty(),
        "the loopback answer must round-trip as zero overlay nodes"
    );
}

#[tokio::test]
async fn quic_server_accepts_connection_and_reports_short_id_sni() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();

    let client_handle = tokio::spawn(async move {
        QuicSession::connect(
            "127.0.0.1:0".parse().unwrap(),
            server_addr,
            client_keypair,
            server_keypair.public_key,
        )
        .await
        .unwrap()
    });

    let (_connection, sni) = server.accept().await.unwrap();
    assert_eq!(
        sni,
        Some(sni_for_public_key(&server_keypair.public_key)),
        "the server must observe the peer's short-id SNI"
    );
    let client = client_handle.await.unwrap();
    assert_eq!(
        client.connection().remote_address(),
        server.endpoint.local_addr().unwrap()
    );
}

#[tokio::test]
async fn quic_query_rejects_invalid_answer_frame() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();

    let client_handle = tokio::spawn(async move {
        QuicSession::connect(
            "127.0.0.1:0".parse().unwrap(),
            server_addr,
            client_keypair,
            server_keypair.public_key,
        )
        .await
        .unwrap()
    });

    let (server_conn, _) = server.accept().await.unwrap();
    let server_fut = async move {
        let (mut send, mut recv) = server_conn.accept_bi().await.unwrap();
        // Validate inbound framing, then reply with an unknown
        // constructor instead of a `quic.answer` frame.
        let _query: QuicQuery =
            tl_proto::deserialize(&recv.read_to_end(MAX_FRAME_SIZE).await.unwrap()).unwrap();
        send.write_all(&[0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00])
            .await
            .unwrap();
        send.finish().unwrap();
        // Keep the connection open until the client reads the frame,
        // so the client observes the invalid frame rather than a
        // transport-level connection loss.
        server_conn.closed().await;
    };

    let client_fut = async move {
        let client = client_handle.await.unwrap();
        client
            .send_query(vec![1, 2, 3], Duration::from_secs(5))
            .await
    };

    let ((), result) = tokio::join!(server_fut, client_fut);
    assert!(
        matches!(result, Err(AdnlError::InvalidQuicAnswer(_))),
        "non-quic.answer reply must fail closed with InvalidQuicAnswer, got {result:?}"
    );
}

#[tokio::test]
async fn quic_client_rejects_mismatched_server_key() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let wrong_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng).public_key;

    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();

    let client_handle = tokio::spawn(async move {
        QuicSession::connect(
            "127.0.0.1:0".parse().unwrap(),
            server_addr,
            client_keypair,
            wrong_key,
        )
        .await
    });

    let (client_result, server_result) = tokio::join!(
        client_handle,
        tokio::time::timeout(Duration::from_secs(10), server.accept())
    );
    assert!(
        client_result.unwrap().is_err(),
        "the client must fail when the server key differs from the dialed key"
    );
    let server_result = server_result.expect("server must fail before the timeout");
    assert!(
        server_result.is_err(),
        "the server must not accept a connection whose peer aborted the handshake"
    );
}

#[tokio::test]
async fn quic_server_rejects_client_without_raw_public_keys() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let expected = server_keypair.public_key;

    let client_handle = tokio::spawn(async move {
        let config = make_quinn_client_config_without_identity(&expected).unwrap();
        let sni = sni_for_public_key(&expected);
        connect_raw_client(config, server_addr, &sni).await
    });

    let (client_result, server_result) = tokio::join!(
        client_handle,
        tokio::time::timeout(Duration::from_secs(10), server.accept())
    );
    assert!(
        client_result.unwrap().is_err(),
        "a client that cannot present an Ed25519 raw public key must not complete \
     the handshake: upstream requires a peer key (parse_peer_id) and this \
     server mirrors it with a mandatory client certificate"
    );
    let server_result = server_result.expect("server must fail before the timeout");
    assert!(
        server_result.is_err(),
        "the server must not accept a client with no identity"
    );
}

#[tokio::test]
async fn quic_server_rejects_mismatched_sni() {
    let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair)
        .await
        .unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let expected = server_keypair.public_key;

    let client_handle = tokio::spawn(async move {
        let config = make_quinn_client_config(&client_keypair, &expected).unwrap();
        // Syntactically valid DNS name that is not the server's short id,
        // the case upstream's SNI callback rejects with `unrecognized_name`.
        let sni = format!("{}.{}.adnl", "0".repeat(32), "f".repeat(32));
        connect_raw_client(config, server_addr, &sni).await
    });

    let (client_result, server_result) = tokio::join!(
        client_handle,
        tokio::time::timeout(Duration::from_secs(10), server.accept())
    );
    assert!(
        client_result.unwrap().is_err(),
        "a client whose SNI is not the server's short id must not complete \
     the handshake"
    );
    let server_result = server_result.expect("server must fail before the timeout");
    assert!(
        server_result.is_err(),
        "the server must not resolve its identity for an unrecognized SNI"
    );
}
