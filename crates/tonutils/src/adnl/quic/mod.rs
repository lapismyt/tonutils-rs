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

pub mod tls;

#[cfg(test)]
mod tests;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    adnl::{
        AdnlError, AdnlKeyPair, AdnlPublicKey,
        quic::tls::{make_quinn_client_config, make_quinn_server_config, sni_for_public_key},
    },
    tl::{
        Int256,
        network::{
            DhtMessage, DhtNodesBoxed, DhtValueResult, OverlayNodes, OverlayNodesBoxed,
            OverlayQuery, QuicAnswer, QuicMessage, QuicQuery,
        },
    },
};

/// Default maximum QUIC stream buffer size (1 MiB).
const MAX_FRAME_SIZE: usize = 1 << 20;

/// QUIC connection wrapper for TON protocol.
pub struct QuicSession {
    endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    local_keypair: AdnlKeyPair,
    remote_key: AdnlPublicKey,
}

impl QuicSession {
    /// Connects to a remote TON QUIC endpoint.
    pub async fn connect(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local_keypair: AdnlKeyPair,
        remote_key: AdnlPublicKey,
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
    ) -> Result<DhtNodesBoxed, AdnlError> {
        let query = tl_proto::serialize(DhtMessage::FindNode { key, k: count });
        let response = self.send_query(query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Performs a DHT findValue query over QUIC.
    pub async fn dht_find_value(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtValueResult, AdnlError> {
        let query = tl_proto::serialize(DhtMessage::FindValue { key, k: count });
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
    ) -> Result<OverlayNodesBoxed, AdnlError> {
        let mut query = tl_proto::serialize(OverlayQuery::Query { overlay });
        query.extend(tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: OverlayNodes { nodes: Vec::new() },
        }));
        let response = self.send_query(query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Returns the remote peer's public key.
    pub fn remote_public_key(&self) -> &AdnlPublicKey {
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
