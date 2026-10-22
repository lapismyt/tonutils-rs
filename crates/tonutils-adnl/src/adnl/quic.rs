//! TON QUIC transport layer.
//!
//! Implements the TON QUIC protocol using `quinn` on top of the TLS 1.3
//! raw public key profile in the `quic_tls` module: Ed25519 identities in
//! both directions, ALPN `ton`, and upstream-compatible SNI. TON nodes
//! prefer QUIC over ADNL UDP for inter-node communication.
//!
//! ## Protocol Details
//!
//! - **ALPN**: `"ton"`
//! - **SNI**: `<hex[0:32]>.<hex[32:64]>.adnl`, the lowercase hex of the
//!   peer's ADNL short id (SHA-256 over the TL-tagged Ed25519 public key)
//!   split at 32 characters - see [`sni_for_public_key`]
//! - **Authentication**: each endpoint presents its Ed25519 identity key as
//!   an RFC 7250 raw public key certificate; the client pins the server to
//!   the expected key and the server requires a client raw public key
//!   (upstream's `parse_peer_id` drops a peer key that is missing or not
//!   32 bytes)
//! - **Framing**: `quic.message`, `quic.query`, `quic.answer` (upstream
//!   `ton_api.tl` definitions, `data:bytes` only) on bidirectional streams;
//!   query/answer correlation is per-stream, the frames carry no query id

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tonutils_tl::tl::network::{QuicAnswer, QuicMessage, QuicQuery};

use super::crypto::{KeyPair as AdnlKeyPair, PublicKey};
use super::helper_types::AdnlError;
use super::quic_tls::{make_quinn_client_config, make_quinn_server_config};
use tonutils_tl::Int256;

// Re-exported so `tonutils_adnl::adnl::quic::sni_for_public_key` keeps
// resolving; the derivation itself belongs to the TLS profile module.
pub use super::quic_tls::sni_for_public_key;

/// Default maximum QUIC stream buffer size (1 MiB).
const MAX_FRAME_SIZE: usize = 1 << 20;

/// QUIC connection wrapper for TON protocol.
pub struct QuicSession {
    #[allow(dead_code)]
    endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    #[allow(dead_code)]
    local_keypair: AdnlKeyPair,
    remote_key: PublicKey,
}

impl QuicSession {
    /// Connects to a remote TON QUIC endpoint.
    pub async fn connect(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local_keypair: AdnlKeyPair,
        remote_key: PublicKey,
    ) -> Result<Self, AdnlError> {
        // Upstream servers select their identity from this short-id SNI and
        // fail the handshake on a mismatch, so the client must send the
        // short id of the key it dials.
        let sni = sni_for_public_key(&remote_key);

        let quinn_config = make_quinn_client_config(&local_keypair, &remote_key)?;

        let socket = std::net::UdpSocket::bind(local_addr)?;
        socket.set_nonblocking(true)?;
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        let connection = endpoint
            .connect_with(quinn_config, remote_addr, &sni)
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        Ok(Self {
            endpoint,
            connection,
            local_keypair,
            remote_key,
        })
    }

    /// Sends a `quic.query` on a fresh bidirectional stream and waits for the
    /// matching `quic.answer` on the same stream. Correlation is per-stream;
    /// the upstream `quic.query`/`quic.answer` framing carries no query id.
    pub async fn send_query(&self, data: Vec<u8>, timeout: Duration) -> Result<Vec<u8>, AdnlError> {
        let query = QuicQuery { data };
        let (mut send, mut recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        // Write quic.query frame and close send half
        let query_bytes = tl_proto::serialize(query);
        send.write_all(&query_bytes)
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
        send.finish()
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        // Read quic.answer frame
        let buf = tokio::time::timeout(timeout, recv.read_to_end(MAX_FRAME_SIZE))
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "QUIC query",
                timeout,
            })?
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        let answer: QuicAnswer = tl_proto::deserialize(&buf)
            .map_err(|error| AdnlError::InvalidQuicAnswer(error.to_string()))?;
        Ok(answer.data)
    }

    /// Sends a `quic.message` (fire and forget).
    pub async fn send_message(&self, data: Vec<u8>) -> Result<(), AdnlError> {
        let message = QuicMessage { data };
        let (mut send, _recv) = self
            .connection
            .open_bi()
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        let msg_bytes = tl_proto::serialize(message);
        send.write_all(&msg_bytes)
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
        send.finish()
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        Ok(())
    }

    /// Performs a DHT findNode query over QUIC.
    pub async fn dht_find_node(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<tonutils_tl::tl::network::DhtNodesBoxed, AdnlError> {
        let query =
            tl_proto::serialize(tonutils_tl::tl::network::DhtMessage::FindNode { key, k: count });
        let response = self.send_query(query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Performs a DHT findValue query over QUIC.
    pub async fn dht_find_value(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<tonutils_tl::tl::network::DhtValueResult, AdnlError> {
        let query =
            tl_proto::serialize(tonutils_tl::tl::network::DhtMessage::FindValue { key, k: count });
        let response = self.send_query(query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Sends an overlay getRandomPeers query over QUIC.
    ///
    /// `quic.query.data` must carry the `overlay.query{overlay}` header -
    /// upstream `ton_api.tl` defines `overlay.query overlay:int256 = True;`
    /// as a 4-byte constructor plus the 32-byte overlay id, with the inner
    /// query bytes appended RAW (no `bytes` length prefix) - because the
    /// member dispatches on that prefix: `OverlayManager` subscribes only
    /// `overlay.query`/`overlay.queryWithExtra` (plus the two message
    /// constructors) on the member's NODE id, and a bare inner query
    /// matches no prefix, so it is dropped with "no callbacks for custom
    /// query" and `QuicSender::on_inbound_query` has no error branch - no
    /// `quic.answer` is sent and the client just times out. That was the
    /// recorded pre-fix symptom: 4/4 completed mainnet handshakes never
    /// answered `overlay.getRandomPeers` within 15s (Evidence in
    /// `docs/reference/network/quic.md`).
    ///
    /// The framing matches the proven UDP path
    /// (`AdnlUdpSession::send_overlay_get_random_peers_with_id`) and
    /// upstream's `OverlayManager::send_query_via`.
    ///
    /// `peers` stays empty: the QUIC session has no signed local overlay
    /// node (the UDP client builds one from its channel address list), and
    /// that is safe because upstream `OverlayImpl::process_query` answers
    /// `overlay.getRandomPeers` regardless of the list - `add_peers` of an
    /// empty `overlay.nodes` is a no-op. The only cost is peer admission,
    /// not the answer: an outgoing query with an empty `peers` list does
    /// not queue this node in the member's `pending_peers_` (see
    /// `docs/reference/network/overlay.md`), so ping-back visibility is
    /// left to the UDP path that carries the signed record.
    pub async fn overlay_get_random_peers(
        &self,
        overlay: Int256,
        timeout: Duration,
    ) -> Result<tonutils_tl::tl::network::OverlayNodesBoxed, AdnlError> {
        let mut query =
            tl_proto::serialize(tonutils_tl::tl::network::OverlayQuery::Query { overlay });
        query.extend(tl_proto::serialize(
            tonutils_tl::tl::network::OverlayQuery::GetRandomPeers {
                peers: tonutils_tl::tl::network::OverlayNodes { nodes: Vec::new() },
            },
        ));
        let response = self.send_query(query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Returns the remote peer's public key.
    pub fn remote_public_key(&self) -> &PublicKey {
        &self.remote_key
    }

    /// Returns the QUIC connection.
    pub fn connection(&self) -> &quinn::Connection {
        &self.connection
    }
}

/// QUIC server that accepts incoming TON QUIC connections.
///
/// The handshake is gated by the TLS profile: the server presents the node
/// identity only for the node's own short-id SNI and requires the client to
/// present an Ed25519 raw public key, matching upstream's SNI callback and
/// peer-key checks.
pub struct QuicServer {
    endpoint: quinn::Endpoint,
    #[allow(dead_code)]
    local_keypair: AdnlKeyPair,
}

impl QuicServer {
    /// Creates a new QUIC server listening on the given address.
    pub fn bind(local_addr: SocketAddr, local_keypair: AdnlKeyPair) -> Result<Self, AdnlError> {
        let server_config = make_quinn_server_config(&local_keypair)?;

        let socket = std::net::UdpSocket::bind(local_addr)?;
        socket.set_nonblocking(true)?;
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(server_config),
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        Ok(Self {
            endpoint,
            local_keypair,
        })
    }

    /// Returns the address the server is listening on.
    ///
    /// Required to discover the bound port when `bind` was
    /// given a zero port.
    pub fn local_addr(&self) -> Result<SocketAddr, AdnlError> {
        self.endpoint
            .local_addr()
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Accepts the next incoming QUIC connection.
    ///
    /// Returns the connection and the SNI the peer sent, if any. A peer
    /// whose SNI is not this node's short id already failed inside the
    /// handshake (see `quic_tls::SniRpkResolver`), so a returned connection
    /// completed the full raw-public-key handshake.
    pub async fn accept(&self) -> Result<(quinn::Connection, Option<String>), AdnlError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| AdnlError::TlsConfig("QUIC server closed".into()))?;

        let connecting = incoming
            .accept()
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        let connection = connecting
            .await
            .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

        let server_name = connection.handshake_data().as_ref().and_then(|data| {
            data.downcast_ref::<quinn::crypto::rustls::HandshakeData>()
                .and_then(|hs| hs.server_name.clone())
        });

        Ok((connection, server_name))
    }
}

#[cfg(test)]
mod tests {
    use super::super::quic_tls::make_quinn_client_config_without_identity;
    use super::*;

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

        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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

    #[tokio::test]
    async fn overlay_get_random_peers_wraps_the_overlay_query_header() {
        let server_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let client_keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);

        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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
            let mut expected = tl_proto::serialize(tonutils_tl::tl::network::OverlayQuery::Query {
                overlay: Int256([7; 32]),
            });
            expected.extend(tl_proto::serialize(
                tonutils_tl::tl::network::OverlayQuery::GetRandomPeers {
                    peers: tonutils_tl::tl::network::OverlayNodes { nodes: vec![] },
                },
            ));
            assert_eq!(
                query.data, expected,
                "the quic.query payload must be the overlay.query header \
                 (constructor + overlay id) followed by the raw \
                 overlay.getRandomPeers query"
            );

            let answer = QuicAnswer {
                data: tl_proto::serialize(tonutils_tl::tl::network::OverlayNodesBoxed {
                    nodes: vec![],
                }),
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

        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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
        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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

        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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
        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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
        let server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), server_keypair).unwrap();
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
}
