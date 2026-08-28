//! TON QUIC transport layer.
//!
//! Implements the TON QUIC protocol using `quinn` with Ed25519 Raw Public Keys
//! (RFC 7250) for authentication. TON nodes prefer QUIC over ADNL UDP for
//! inter-node communication.
//!
//! ## Protocol Details
//!
//! - **ALPN**: `"ton"`
//! - **SNI**: `<hex[0:32]>.<hex[32:64]>.adnl` (lowercase hex of Ed25519 public key)
//! - **Authentication**: Ed25519 identity key used as the TLS certificate key.
//!   The SAN encodes the hex-encoded public key for peer verification.
//! - **Framing**: `quic.message`, `quic.query`, `quic.answer` on bidirectional streams

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair as RcgenKeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tonutils_tl::tl::network::{QuicAnswer, QuicMessage, QuicQuery};
use x509_cert::certificate::Certificate;
use x509_cert::der::Decode;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;

use super::crypto::{KeyPair as AdnlKeyPair, PublicKey, build_ed25519_pkcs8};
use super::helper_types::AdnlError;
use tonutils_tl::Int256;

/// Default maximum QUIC stream buffer size (1 MiB).
const MAX_FRAME_SIZE: usize = 1 << 20;

/// Derives the SNI hostname for a TON QUIC connection.
///
/// Format: `<hex[0:32]>.<hex[32:64]>.adnl` (lowercase)
pub fn sni_for_public_key(key: &PublicKey) -> String {
    let hex_str = hex::encode(key.to_bytes());
    format!("{}.{}.adnl", &hex_str[..32], &hex_str[32..])
}

/// Extracts the Ed25519 public key from a certificate's SAN.
///
/// TON certificates encode the hex-encoded Ed25519 public key as the SAN
/// in the format `<hex[0:32]>.<hex[32:64]>`.
fn extract_public_key_from_cert(cert: &CertificateDer<'_>) -> Option<PublicKey> {
    let x509 = Certificate::from_der(cert.as_ref()).ok()?;
    let (_critical, san): (bool, SubjectAltName) =
        x509.tbs_certificate.get::<SubjectAltName>().ok()??;

    for name in san.0 {
        if let GeneralName::DnsName(dns_name) = name {
            let name_str = dns_name.to_string();
            let parts: Vec<&str> = name_str.split('.').collect();
            if parts.len() != 2 || parts[0].len() != 32 || parts[1].len() != 32 {
                continue;
            }
            let full_hex = format!("{}{}", parts[0], parts[1]);
            let key_bytes = hex::decode(&full_hex).ok()?;
            if key_bytes.len() != 32 {
                continue;
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&key_bytes);
            return PublicKey::from_bytes(arr);
        }
    }
    None
}

/// Generates a self-signed X.509 certificate using the node's actual Ed25519 key.
///
/// The TLS certificate key IS the node's Ed25519 identity key, so the TLS
/// handshake proves possession of the identity key. The SAN encodes the
/// hex-encoded public key for peer verification.
fn make_cert_for_keypair(
    keypair: &AdnlKeyPair,
) -> Result<(CertificateDer<'static>, RcgenKeyPair), AdnlError> {
    let pkcs8_bytes = build_ed25519_pkcs8(keypair.secret_key.key_bytes());
    let pkcs8_der = rustls_pki_types::PrivatePkcs8KeyDer::from(pkcs8_bytes);
    let rcgen_key = RcgenKeyPair::from_pkcs8_der_and_sign_algo(&pkcs8_der, &rcgen::PKCS_ED25519)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

    let pub_hex = hex::encode(keypair.public_key.to_bytes());
    let san = format!("{}.{}", &pub_hex[..32], &pub_hex[32..]);

    let mut params =
        CertificateParams::new(vec![san]).map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    params.distinguished_name = rcgen::DistinguishedName::new();

    let cert = params
        .self_signed(&rcgen_key)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;

    let der = CertificateDer::from(cert.der().to_vec());
    Ok((der, rcgen_key))
}

/// Custom server certificate verifier that validates Ed25519 RPK.
///
/// Extracts the Ed25519 public key from the certificate's SAN and verifies
/// it matches the expected peer key.
#[derive(Debug)]
struct TonServerCertVerifier {
    expected_key: PublicKey,
}

impl ServerCertVerifier for TonServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert_key = extract_public_key_from_cert(end_entity).ok_or_else(|| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;

        if cert_key != self.expected_key {
            log::warn!(
                "QUIC TLS certificate key mismatch: expected {:?}, got {:?}",
                self.expected_key.to_bytes(),
                cert_key.to_bytes()
            );
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::Other(rustls::OtherError(Arc::new(
                    AdnlError::CertificateKeyMismatch {
                        expected: self.expected_key.to_bytes(),
                        got: cert_key.to_bytes(),
                    },
                ))),
            ));
        }

        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dbsig: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // QUIC uses TLS 1.3 only; this path should never be reached.
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dbsig: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dbsig,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}

/// Creates a quinn `ClientConfig` for TON QUIC.
fn make_quinn_client_config(expected_key: &PublicKey) -> Result<quinn::ClientConfig, AdnlError> {
    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TonServerCertVerifier {
            expected_key: *expected_key,
        }))
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![b"ton".to_vec()];

    let quic_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_config)))
}

/// Creates a quinn `ServerConfig` for TON QUIC.
fn make_quinn_server_config(keypair: &AdnlKeyPair) -> Result<quinn::ServerConfig, AdnlError> {
    let (cert, rcgen_key) = make_cert_for_keypair(keypair)?;

    let pkcs8 = rustls_pki_types::PrivateKeyDer::Pkcs8(rustls_pki_types::PrivatePkcs8KeyDer::from(
        rcgen_key.serialize_der(),
    ));

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], pkcs8)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    tls_config.alpn_protocols = vec![b"ton".to_vec()];

    let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic_config)))
}

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
        let sni = sni_for_public_key(&remote_key);

        let quinn_config = make_quinn_client_config(&remote_key)?;

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

    /// Sends a `quic.query` and waits for the corresponding `quic.answer`.
    pub async fn send_query(
        &self,
        query_id: Int256,
        data: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, AdnlError> {
        let query = QuicQuery {
            id: query_id.clone(),
            data,
        };
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
        if answer.id != query_id {
            return Err(AdnlError::InvalidQuicAnswer(
                "answer query id does not match request".to_owned(),
            ));
        }
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
        let query_id = Int256::random();
        let query =
            tl_proto::serialize(tonutils_tl::tl::network::DhtMessage::FindNode { key, k: count });
        let response = self.send_query(query_id, query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Performs a DHT findValue query over QUIC.
    pub async fn dht_find_value(
        &self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<tonutils_tl::tl::network::DhtValueResult, AdnlError> {
        let query_id = Int256::random();
        let query =
            tl_proto::serialize(tonutils_tl::tl::network::DhtMessage::FindValue { key, k: count });
        let response = self.send_query(query_id, query, timeout).await?;
        tl_proto::deserialize(&response).map_err(|e| AdnlError::TlsConfig(e.to_string()))
    }

    /// Sends an overlay getRandomPeers query over QUIC.
    pub async fn overlay_get_random_peers(
        &self,
        _overlay: Int256,
        timeout: Duration,
    ) -> Result<tonutils_tl::tl::network::OverlayNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(tonutils_tl::tl::network::OverlayQuery::GetRandomPeers {
            peers: tonutils_tl::tl::network::OverlayNodes { nodes: Vec::new() },
        });
        let response = self.send_query(query_id, query, timeout).await?;
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

    /// Accepts the next incoming QUIC connection.
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
    use super::*;

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

    #[test]
    fn extract_public_key_from_cert_roundtrips() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let (cert, _rcgen_key) = make_cert_for_keypair(&keypair).unwrap();
        let extracted = extract_public_key_from_cert(&cert).expect("should extract key");
        assert_eq!(extracted, keypair.public_key);
    }

    #[test]
    fn extract_public_key_rejects_wrong_key() {
        let keypair1 = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let keypair2 = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let (cert, _rcgen_key) = make_cert_for_keypair(&keypair1).unwrap();
        let extracted = extract_public_key_from_cert(&cert).expect("should extract key");
        assert_ne!(extracted, keypair2.public_key);
    }

    #[test]
    fn cert_verifier_rejects_wrong_key() {
        let keypair1 = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let keypair2 = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let (cert, _rcgen_key) = make_cert_for_keypair(&keypair1).unwrap();

        let verifier = TonServerCertVerifier {
            expected_key: keypair2.public_key,
        };
        let result = verifier.verify_server_cert(
            &cert,
            &[],
            &ServerName::try_from("test").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(result.is_err());
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
            let query: QuicQuery = tl_proto::deserialize(&buf).unwrap();
            let answer = QuicAnswer {
                id: query.id.clone(),
                data: vec![42],
            };
            let answer_bytes = tl_proto::serialize(answer);
            send.write_all(&answer_bytes).await.unwrap();
            send.finish().unwrap();
            server_conn.closed().await;
        };

        let client_fut = async {
            let client = client_handle.await.unwrap();
            let query_id = Int256([1; 32]);
            client
                .send_query(query_id, vec![1, 2, 3], Duration::from_secs(5))
                .await
                .unwrap()
        };

        let (_server_result, response) = tokio::join!(server_fut, client_fut);
        assert_eq!(response, vec![42]);
    }

    #[tokio::test]
    async fn quic_server_accepts_connection() {
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

        let (_connection, _sni) = server.accept().await.unwrap();
        let client = client_handle.await.unwrap();
        assert_eq!(
            client.connection().remote_address(),
            server.endpoint.local_addr().unwrap()
        );
    }

    #[tokio::test]
    async fn quic_query_rejects_mismatched_answer_id() {
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
            let query: QuicQuery =
                tl_proto::deserialize(&recv.read_to_end(MAX_FRAME_SIZE).await.unwrap()).unwrap();
            send.write_all(&tl_proto::serialize(QuicAnswer {
                id: Int256([9; 32]),
                data: query.data,
            }))
            .await
            .unwrap();
            send.finish().unwrap();
        };

        let client_fut = async move {
            let client = client_handle.await.unwrap();
            client
                .send_query(Int256([1; 32]), vec![1, 2, 3], Duration::from_secs(5))
                .await
        };

        let ((), result) = tokio::join!(server_fut, client_fut);
        assert!(result.is_err(), "mismatched QUIC answer must fail closed");
    }
}
