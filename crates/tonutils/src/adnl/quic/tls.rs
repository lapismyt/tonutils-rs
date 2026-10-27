//! TON QUIC TLS profile: TLS 1.3 with Ed25519 raw public keys (RFC 7250).
//!
//! Upstream `ton` terminates QUIC TLS with OpenSSL/ngtcp2, TLS 1.3 only, ALPN
//! `ton`, and RFC 7250 raw public keys in both directions:
//! `quic/quic-pimpl.cpp::setup_rpk_context` pins the protocol to TLS 1.3 and
//! sets `SSL_CTX_set1_server_cert_type` and `SSL_CTX_set1_client_cert_type`
//! to `TLSEXT_cert_type_rpk`, so each endpoint presents its Ed25519 identity
//! key as the certificate payload and proves possession with a standard
//! Ed25519 `CertificateVerify`. Upstream requests a client certificate
//! (`SSL_VERIFY_PEER`) and drops connections whose peer key is absent or not
//! 32 bytes (`quic/quic-pimpl.cpp::on_connected` -> `parse_peer_id`); this
//! module mirrors that with a mandatory client raw public key.
//!
//! Upstream selects the server identity by SNI: `ServerIdentity::sni`
//! (`quic/quic-common.h`) formats the ADNL short id as
//! `<hex[0:32]>.<hex[32:64]>.adnl`, and a name that matches no known
//! identity fails the handshake with a fatal `unrecognized_name` alert.
//! [`SniRpkResolver`] reproduces that rule for rustls: the certificate
//! resolver is the only `ClientHello`-time hook rustls offers, so a
//! mismatched SNI yields no server certificate and the handshake dies with a
//! fatal alert, while a missing SNI keeps the default identity - the same
//! fallback OpenSSL applies when a client sends no SNI.
//!
//! Certificate verification accepts only a 44-byte Ed25519
//! SubjectPublicKeyInfo (RFC 8410 prefix `302a300506032b6570032100`) or a
//! 32-byte raw key, matching upstream's `SSL_get0_peer_rpk` extraction.
//! There is deliberately no X.509 chain or SAN fallback.
//!
//! This is the same rustls configuration shape as the upstream reference
//! implementation `test/quic-interop-rust`, which pairs the C++ example
//! client and server with quinn + rustls raw public keys.

use std::sync::Arc;

use crate::adnl::{AdnlAddress, AdnlError, AdnlKeyPair, AdnlPublicKey};

use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::{
        ClientHello, ResolvesServerCert,
        danger::{ClientCertVerified, ClientCertVerifier},
    },
    sign::CertifiedKey,
};

/// SubjectPublicKeyInfo prefix for an Ed25519 public key (RFC 8410):
/// `SEQUENCE { OID 1.3.101.112, BIT STRING (32 bytes) }`.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// ALPN protocol id upstream negotiates for TON QUIC
/// (`quic/quic-pimpl.cpp::QuicServer`, `alpn = "ton"`).
const ALPN_TON: &[u8] = b"ton";

/// Derives the SNI hostname for a TON QUIC connection.
///
/// The hostname is the peer's ADNL short id - SHA-256 over the TL-tagged
/// Ed25519 public key (`pub.ed25519`, constructor id `0x4813b4c6`) - as
/// lowercase hex split at 32 characters with an `.adnl` suffix:
/// `<hex[0:32]>.<hex[32:64]>.adnl`.
///
/// Upstream derives SNI from the short id, not from the raw key
/// (`ServerIdentity::sni` in `quic/quic-common.h`), and servers reject an
/// unknown name during the handshake (`quic/quic-pimpl.cpp`), so this
/// derivation must stay byte-exact to interoperate.
#[must_use]
pub fn sni_for_public_key(key: &AdnlPublicKey) -> String {
    let short_id = AdnlAddress::from(key);
    let hex_str = hex::encode(short_id.to_bytes());
    format!("{}.{}.adnl", &hex_str[..32], &hex_str[32..])
}

/// Extracts the Ed25519 public key from a raw public key certificate payload.
///
/// Accepts the RFC 8410 SubjectPublicKeyInfo encoding (44 bytes) and the
/// bare 32-byte key encoding; anything else - including a full X.509
/// container - yields `None`, mirroring upstream's `SSL_get0_peer_rpk`
/// result, which only carries a key when the negotiated certificate type is
/// raw public key.
fn extract_ed25519_key(cert: &CertificateDer<'_>) -> Option<AdnlPublicKey> {
    let bytes = cert.as_ref();
    let raw = match bytes.len() {
        44 if bytes[..12] == ED25519_SPKI_PREFIX => &bytes[12..],
        32 => bytes,
        _ => return None,
    };
    let mut arr = [0u8; 32];
    arr.copy_from_slice(raw);
    AdnlPublicKey::from_bytes(arr)
}

/// Builds the TLS certificate that presents `keypair`'s Ed25519 identity as
/// a raw public key: the certificate entry is the key's own SPKI DER and the
/// signing key is the identity key itself, so the handshake `CertificateVerify`
/// proves possession of the ADNL identity key.
fn make_certified_key(keypair: &AdnlKeyPair) -> Result<Arc<CertifiedKey>, AdnlError> {
    let provider = rustls::crypto::ring::default_provider();
    let signing_key = provider
        .key_provider
        .load_private_key(keypair.private_key_der())
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    let spki = signing_key
        .public_key()
        .ok_or_else(|| AdnlError::TlsConfig("Ed25519 signing key exposes no SPKI".into()))?;
    let cert = CertificateDer::from(spki.as_ref().to_vec());
    Ok(Arc::new(CertifiedKey::new(vec![cert], signing_key)))
}

/// Verifies an Ed25519 TLS 1.3 `CertificateVerify` signature over `message`.
///
/// The signature is checked as standard RFC 8032 Ed25519 against the key
/// extracted from `cert`; QUIC never negotiates TLS 1.2, so no other scheme
/// is accepted.
fn verify_ed25519_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    scheme: SignatureScheme,
    signature: &[u8],
) -> Result<(), rustls::Error> {
    if scheme != SignatureScheme::ED25519 {
        return Err(rustls::Error::General(format!(
            "unsupported QUIC handshake signature scheme {scheme:?}"
        )));
    }
    let key = extract_ed25519_key(cert)
        .ok_or_else(|| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    let sig: [u8; 64] = signature
        .try_into()
        .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
    if key.verify_raw(message, &sig) {
        return Ok(());
    }
    log::warn!(
        "QUIC handshake CertificateVerify failed for key {:?}",
        key.to_bytes()
    );
    Err(rustls::Error::General(
        "invalid Ed25519 handshake signature".into(),
    ))
}

/// Client-side verifier: the server's raw public key must equal the key the
/// caller expects for this peer, which pins the connection to the ADNL
/// identity the client dialed.
#[derive(Debug)]
pub(crate) struct TonServerCertVerifier {
    pub(crate) expected_key: AdnlPublicKey,
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
        let cert_key = extract_ed25519_key(end_entity).ok_or_else(|| {
            log::warn!("QUIC server presented no parseable Ed25519 raw public key");
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
        _dbsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Upstream pins QUIC to TLS 1.3
        // (`quic/quic-pimpl.cpp::setup_rpk_context`), so a TLS 1.2
        // signature must never validate.
        Err(rustls::Error::General(
            "QUIC requires TLS 1.3 handshakes".into(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dbsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_ed25519_signature(message, cert, dbsig.scheme, dbsig.signature())?;
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Server-side verifier: every client must present an Ed25519 raw public key
/// and prove possession of it. This mirrors upstream, which requests a
/// client certificate and rejects connections whose peer key is missing
/// after the handshake (`quic/quic-pimpl.cpp::parse_peer_id`); here the
/// rejection happens inside the handshake instead.
#[derive(Debug)]
pub(crate) struct TonRpkClientVerifier;

impl ClientCertVerifier for TonRpkClientVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        if extract_ed25519_key(end_entity).is_some() {
            return Ok(ClientCertVerified::assertion());
        }
        log::warn!("QUIC client presented no parseable Ed25519 raw public key");
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadEncoding,
        ))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dbsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(
            "QUIC requires TLS 1.3 handshakes".into(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dbsig: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_ed25519_signature(message, cert, dbsig.scheme, dbsig.signature())?;
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

/// Server certificate resolver that presents the node identity only when the
/// client's SNI matches the node's ADNL short id, reproducing upstream's
/// SNI-selection callback: an unknown name fails the handshake (rustls sends
/// a fatal alert when no certificate resolves), while no SNI keeps the
/// default identity, as OpenSSL does.
#[derive(Debug)]
pub(crate) struct SniRpkResolver {
    identity: Arc<CertifiedKey>,
    expected_sni: String,
}

impl ResolvesServerCert for SniRpkResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        match client_hello.server_name() {
            None => Some(self.identity.clone()),
            Some(name) if name == self.expected_sni => Some(self.identity.clone()),
            Some(name) => {
                log::warn!(
                    "rejected QUIC peer with unrecognized SNI {name:?}, expected {:?}",
                    self.expected_sni
                );
                None
            }
        }
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }
}

/// Creates the quinn `ClientConfig` for TON QUIC: TLS 1.3 only, ALPN `ton`,
/// the server certificate pinned to `expected_key`, and the local identity
/// presented as a raw public key so upstream servers can authenticate the
/// client (`SSL_VERIFY_PEER` + `parse_peer_id`).
pub(crate) fn make_quinn_client_config(
    local_keypair: &AdnlKeyPair,
    expected_key: &AdnlPublicKey,
) -> Result<quinn::ClientConfig, AdnlError> {
    let mut tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| AdnlError::TlsConfig(e.to_string()))?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(TonServerCertVerifier {
        expected_key: *expected_key,
    }))
    .with_client_cert_resolver(Arc::new(
        rustls::client::AlwaysResolvesClientRawPublicKeys::new(make_certified_key(local_keypair)?),
    ));
    tls_config.alpn_protocols = vec![ALPN_TON.to_vec()];

    let quic_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_config)))
}

/// Creates the quinn `ServerConfig` for TON QUIC: TLS 1.3 only, ALPN `ton`,
/// the node identity presented as a raw public key, SNI-gated by
/// [`SniRpkResolver`], and a mandatory client raw public key.
pub(crate) fn make_quinn_server_config(
    keypair: &AdnlKeyPair,
) -> Result<quinn::ServerConfig, AdnlError> {
    make_quinn_server_config_with_alpn(keypair, &[ALPN_TON])
}

/// Same profile as [`make_quinn_server_config`] with a caller-chosen ALPN
/// list.
///
/// Production servers always negotiate `ton` alone; the QUIC interop test
/// harness uses this to additionally offer the upstream demo ALPN
/// (`hq-interop`) when pairing against `quic-example-client`, without
/// changing the shipped server profile.
pub(crate) fn make_quinn_server_config_with_alpn(
    keypair: &AdnlKeyPair,
    alpn: &[&[u8]],
) -> Result<quinn::ServerConfig, AdnlError> {
    let resolver = SniRpkResolver {
        identity: make_certified_key(keypair)?,
        expected_sni: sni_for_public_key(&keypair.public_key),
    };

    let mut tls_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| AdnlError::TlsConfig(e.to_string()))?
    .with_client_cert_verifier(Arc::new(TonRpkClientVerifier))
    .with_cert_resolver(Arc::new(resolver));
    tls_config.alpn_protocols = alpn.iter().map(|proto| proto.to_vec()).collect();

    let quic_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic_config)))
}

/// Test-only client config that verifies the server but cannot present a
/// client identity. A handshake between this config and [`make_quinn_server_config`]
/// fails, which proves the server's mandatory-client-key interlock rather
/// than any transport-level timeout.
#[cfg(test)]
pub(crate) fn make_quinn_client_config_without_identity(
    expected_key: &AdnlPublicKey,
) -> Result<quinn::ClientConfig, AdnlError> {
    let mut tls_config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|e| AdnlError::TlsConfig(e.to_string()))?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(TonServerCertVerifier {
        expected_key: *expected_key,
    }))
    .with_no_client_auth();
    tls_config.alpn_protocols = vec![ALPN_TON.to_vec()];

    let quic_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
        .map_err(|e| AdnlError::TlsConfig(e.to_string()))?;
    Ok(quinn::ClientConfig::new(Arc::new(quic_config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adnl::crypto::SecretKey;

    /// Seed for the fixed SNI vector. The expected public key below comes
    /// from OpenSSL (`openssl pkey -inform der` over the PKCS#8 built from
    /// this seed), and the expected short id from
    /// `sha256(c6b41348 || pubkey)` computed outside this crate (`c6 b4 13 48`
    /// is the little-endian wire form of ctor id `0x4813b4c6`), so the test
    /// fails if either the derivation or the hashing wrapper drifts.
    const SNI_SEED: [u8; 32] = [0x42; 32];
    const SNI_PUBKEY_HEX: &str = "2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12";
    const SNI_SHORT_ID_HEX: &str =
        "c46870c7c81b3b56bafd6e0836362a189ebc58281029effb8460bda50f3a3af0";
    const SNI_EXPECTED: &str =
        "c46870c7c81b3b56bafd6e0836362a18.9ebc58281029effb8460bda50f3a3af0.adnl";

    fn keypair() -> AdnlKeyPair {
        AdnlKeyPair::generate(&mut rand::rngs::OsRng)
    }

    fn spki_cert_for(keypair: &AdnlKeyPair) -> CertificateDer<'static> {
        let mut bytes = ED25519_SPKI_PREFIX.to_vec();
        bytes.extend_from_slice(&keypair.public_key.to_bytes());
        CertificateDer::from(bytes)
    }

    #[test]
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    fn sni_for_public_key_matches_the_adnl_short_id_vector() {
        let keypair = AdnlKeyPair::from(&SecretKey::from_bytes(SNI_SEED));
        assert_eq!(hex::encode(keypair.public_key.to_bytes()), SNI_PUBKEY_HEX);
        assert_eq!(
            hex::encode(AdnlAddress::from(&keypair.public_key).to_bytes()),
            SNI_SHORT_ID_HEX
        );
        assert_eq!(sni_for_public_key(&keypair.public_key), SNI_EXPECTED);
        assert!(SNI_EXPECTED.ends_with(".adnl"));
        let labels: Vec<&str> = SNI_EXPECTED.trim_end_matches(".adnl").split('.').collect();
        assert_eq!(
            labels,
            [
                "c46870c7c81b3b56bafd6e0836362a18",
                "9ebc58281029effb8460bda50f3a3af0"
            ]
        );
    }

    #[test]
    fn extract_ed25519_key_accepts_spki_and_raw_encodings() {
        let keypair = keypair();
        let pk_bytes = keypair.public_key.to_bytes();

        let mut spki = ED25519_SPKI_PREFIX.to_vec();
        spki.extend_from_slice(&pk_bytes);
        assert_eq!(
            extract_ed25519_key(&CertificateDer::from(spki)),
            Some(keypair.public_key)
        );

        assert_eq!(
            extract_ed25519_key(&CertificateDer::from(pk_bytes.to_vec())),
            Some(keypair.public_key)
        );
    }

    #[test]
    fn extract_ed25519_key_rejects_foreign_encodings() {
        // Wrong lengths.
        assert_eq!(
            extract_ed25519_key(&CertificateDer::from(vec![0u8; 31])),
            None
        );
        assert_eq!(
            extract_ed25519_key(&CertificateDer::from(vec![0u8; 45])),
            None
        );

        // A foreign SubjectPublicKeyInfo prefix.
        let mut bytes = ED25519_SPKI_PREFIX;
        bytes[1] = 0x31;
        let mut bytes = bytes.to_vec();
        bytes.extend_from_slice(&[0x22; 32]);
        assert_eq!(extract_ed25519_key(&CertificateDer::from(bytes)), None);

        // A y coordinate with no curve point behind it: for y = 2 the
        // value x^2 = (y^2 - 1)/(d*y^2 + 1) is a quadratic non-residue, so
        // the encoding never decompresses (checked against curve25519-dalek's
        // sqrt_ratio_i acceptance rule).
        assert_eq!(
            extract_ed25519_key(&CertificateDer::from({
                let mut non_point = [0u8; 32];
                non_point[0] = 2;
                non_point.to_vec()
            })),
            None
        );

        // An X.509 container: upstream has no SAN fallback either.
        assert_eq!(
            extract_ed25519_key(&CertificateDer::from(vec![0x30u8; 300])),
            None
        );
    }

    #[test]
    fn server_verifier_pins_the_key_and_requires_raw_public_keys() {
        let expected = keypair();
        let other = keypair();
        let verifier = TonServerCertVerifier {
            expected_key: expected.public_key,
        };
        assert!(verifier.requires_raw_public_keys());

        let name = ServerName::try_from("test").expect("server name must parse");
        assert!(
            verifier
                .verify_server_cert(&spki_cert_for(&expected), &[], &name, &[], UnixTime::now())
                .is_ok()
        );
        assert!(
            verifier
                .verify_server_cert(&spki_cert_for(&other), &[], &name, &[], UnixTime::now())
                .is_err()
        );
        assert!(
            verifier
                .verify_server_cert(
                    &CertificateDer::from(vec![0x30u8; 300]),
                    &[],
                    &name,
                    &[],
                    UnixTime::now()
                )
                .is_err()
        );
    }

    #[test]
    fn client_verifier_requires_a_mandatory_ed25519_client_key() {
        let verifier = TonRpkClientVerifier;
        assert!(verifier.requires_raw_public_keys());
        assert!(verifier.offer_client_auth());
        assert!(verifier.client_auth_mandatory());
        assert!(verifier.root_hint_subjects().is_empty());

        assert!(
            verifier
                .verify_client_cert(&spki_cert_for(&keypair()), &[], UnixTime::now())
                .is_ok()
        );
        assert!(
            verifier
                .verify_client_cert(
                    &CertificateDer::from(vec![0x30u8; 64]),
                    &[],
                    UnixTime::now()
                )
                .is_err()
        );
    }

    #[test]
    fn certified_key_binds_the_identity_public_key() {
        let keypair = keypair();
        let certified_key =
            make_certified_key(&keypair).expect("the identity key must build a certified key");

        // The certificate entry is the identity key's SPKI, so pinning the
        // extracted key against the dialed key is meaningful.
        assert_eq!(certified_key.cert.len(), 1);
        assert_eq!(
            extract_ed25519_key(&certified_key.cert[0]),
            Some(keypair.public_key)
        );

        // The loaded signing key must sign as the same identity: a
        // CertificateVerify produced by `rustls` has to verify under
        // `keypair.public_key`, which fails if the PKCS#8 seed and the
        // public key drift apart.
        let signer = certified_key
            .key
            .choose_scheme(&[SignatureScheme::ED25519])
            .expect("the identity key must offer Ed25519");
        let message = b"tls 1.3 certificate verify message";
        let signature = signer.sign(message).expect("signing must succeed");
        let sig: [u8; 64] = signature.as_slice().try_into().expect("64-byte signature");
        assert!(keypair.public_key.verify_raw(message, &sig));
    }

    #[test]
    fn handshake_signatures_verify_as_standard_ed25519() {
        let signing = keypair();
        let cert = spki_cert_for(&signing);
        let message = b"ton quic certificate verify context";

        let signature = signing.sign_raw(message);
        assert!(
            verify_ed25519_signature(message, &cert, SignatureScheme::ED25519, &signature).is_ok()
        );

        // Tampered message.
        assert!(
            verify_ed25519_signature(b"tampered", &cert, SignatureScheme::ED25519, &signature)
                .is_err()
        );
        // Foreign signature scheme (QUIC is TLS 1.3 and Ed25519 only).
        assert!(
            verify_ed25519_signature(
                message,
                &cert,
                SignatureScheme::RSA_PKCS1_SHA256,
                &signature
            )
            .is_err()
        );
        // Signature made under a different identity key.
        let other = keypair();
        let other_signature = other.sign_raw(message);
        assert!(
            verify_ed25519_signature(message, &cert, SignatureScheme::ED25519, &other_signature)
                .is_err()
        );
        // Truncated signature.
        assert!(
            verify_ed25519_signature(message, &cert, SignatureScheme::ED25519, &signature[..63])
                .is_err()
        );
    }

    #[test]
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    fn sni_resolver_builds_the_short_id_identity_gate() {
        let server = keypair();
        let resolver = SniRpkResolver {
            identity: make_certified_key(&server).expect("identity must build"),
            expected_sni: sni_for_public_key(&server.public_key),
        };
        // The resolver must negotiate raw public keys for the server
        // certificate type to reach the client as RFC 7250 instead of X.509.
        assert!(resolver.only_raw_public_keys());
        assert!(resolver.expected_sni.ends_with(".adnl"));
        assert_eq!(
            resolver.expected_sni,
            sni_for_public_key(&server.public_key)
        );
        assert_ne!(
            resolver.expected_sni,
            sni_for_public_key(&keypair().public_key)
        );
    }
}
