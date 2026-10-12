# QUIC TLS and wire profile

## Purpose and scope

This page specifies the handshake and stream framing used by the `quic`
feature of `tonutils-adnl`: TLS version pinning, ALPN, SNI derivation, raw
public key certificates in both directions, and signature verification.
It records upstream `ton` facts (`quic/*.cpp`, `quic/*.h`) next to the
rustls mechanisms that implement them, plus the local and cross-SDK
evidence behind each claim.

Discovery targets and the live reachability question live in
[Overlay protocol design](overlay.md#quic-member-discovery); the claim
summary lives in the [protocol matrix](protocol-matrix.md).

## Transport and framing

- `quinn` 0.11 over UDP with the `ring` TLS provider; `QuicSession::connect`
  for clients, `QuicServer::{bind,accept}` for servers.
- Upstream full nodes listen on `adnl_udp_port + 1000`
  (`quic/quic-sender.h::NODE_PORT_OFFSET`); an advertised
  `adnl.address.quic` record wins over the derived port.
- Queries and answers use the upstream `ton_api.tl` framings
  (`quic.query data:bytes = quic.Request`, `quic.answer data:bytes =
  quic.Response`, `quic.message data:bytes = quic.Message`) on bidirectional
  streams. Correlation is per stream; the frames carry no query id, and the
  answer payload must decode as the expected constructor or the query fails
  closed with `AdnlError::InvalidQuicAnswer`.
- Inbound frames are capped at 1 MiB (`MAX_FRAME_SIZE`).

## TLS profile (wire facts)

| Fact | Value |
| --- | --- |
| Protocol version | TLS 1.3 only (`rustls::version::TLS13`, mirroring upstream `SSL_CTX_set_min/max_proto_version(TLS1_3)` in `quic/quic-pimpl.cpp::setup_rpk_context`) |
| ALPN | `ton` (upstream `QuicServer`/`QuicSender` `alpn = "ton"`) |
| SNI | `<hex[0:32]>.<hex[32:64]>.adnl` - lowercase hex of the peer's ADNL short id, split at 32 characters |
| Short id | SHA-256 over the TL-tagged Ed25519 public key (`pub.ed25519`, constructor id `0xc6b41348`), i.e. `AdnlAddress::from(&PublicKey)` |
| Certificate payload | RFC 7250 raw public key: a 44-byte Ed25519 SubjectPublicKeyInfo (`302a300506032b6570032100` prefix, RFC 8410) or a bare 32-byte key. No X.509 chain, no SAN, no fallback |
| Client certificate | Required. The server offers client auth, marks it mandatory, and requires the raw-public-key certificate type; upstream requests a certificate (`SSL_VERIFY_PEER`) and drops a missing or non-32-byte peer key in `quic/quic-pimpl.cpp::parse_peer_id` |
| CertificateVerify | Standard RFC 8032 Ed25519 over the TLS 1.3 transcript hash, verified with `PublicKey::verify_raw`. TLS 1.2 signature verification returns `Err` (fail-closed) |

### SNI gating

Upstream selects the server identity from the ClientHello name
(`ServerIdentity::sni`, `quic/quic-common.h`) and fails the handshake with a
fatal `unrecognized_name` alert for an unknown name (`quic/quic-pimpl.cpp`
SNI callback), while a client that sends no SNI keeps the default identity.
Our rustls path reproduces both branches in `SniRpkResolver`, the
certificate resolver being the only `ClientHello`-time hook rustls exposes:

- name equals the node's short-id SNI (exact, case-sensitive comparison -
  upstream looks the name up in a map keyed by lowercase hex): identity
  resolves and the handshake continues;
- name differs: no certificate resolves, rustls sends the fatal
  `no server certificate chain resolved` alert (`AccessDenied`), and both
  endpoints observe a failed handshake;
- name absent: the identity resolves, mirroring OpenSSL's default identity.

A fixed vector pins the derivation: seed `[0x42; 32]` yields public key
`2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12`
(verified with `openssl pkey -inform der -pubout`), short id
`c46870c7c81b3b56bafd6e0836362a189ebc58281029effb8460bda50f3a3af0`
(verified with `sha256` over `c6b41348 || pubkey`), SNI
`c46870c7c81b3b56bafd6e0836362a18.9ebc58281029effb8460bda50f3a3af0.adnl`.

### Certificate-type interlock

RFC 7250 lets each side offer the certificate types it understands; the
peer's selection travels in the TLS 1.3 EncryptedExtensions (RFC 7250
ss 4.2 places the selection in the server hello, and EncryptedExtensions is
its TLS 1.3 carrier). rustls implements this as:

- `requires_raw_public_keys() = true` on the client verifier makes the
  client offer `server_certificate_types = [raw_public_key]` in its
  ClientHello and require the server's selection in EncryptedExtensions;
- `requires_raw_public_keys() = true` on the server's client verifier makes
  the server require `client_certificate_types = [raw_public_key]` in the
  ClientHello before it accepts the handshake;
- the raw-public-key resolvers (`AlwaysResolvesClientRawPublicKeys`,
  `AlwaysResolvesServerRawPublicKeys`, wrapped by `SniRpkResolver` on the
  server) make each side present its identity key as the certificate.

Consequences, each covered by a loopback test: a client that offers no raw
public key cannot complete the handshake; a client whose server key differs
from the dialed key fails in `verify_server_cert` with
`AdnlError::CertificateKeyMismatch`; a client whose SNI is not the server's
short id fails during the handshake.

## Upstream fact mapping

| Upstream (`ton-blockchain/ton`) | This crate |
| --- | --- |
| `setup_rpk_context`: TLS 1.3 only, `SSL_CTX_set1_server/client_cert_type(TLSEXT_cert_type_rpk)` | `make_quinn_{client,server}_config`: `with_protocol_versions(&[TLS13])`, `requires_raw_public_keys() = true`, `AlwaysResolves{Client,Server}RawPublicKeys` |
| `alpn = "ton"` | `ALPN_TON` on both configs |
| `ServerIdentity::sni` (short id, split 32/32, `.adnl`) | `sni_for_public_key` |
| SNI callback: fatal `unrecognized_name` for an unknown name, default identity otherwise | `SniRpkResolver::resolve` |
| `SSL_VERIFY_PEER` plus `parse_peer_id` rejecting a missing peer key | `TonRpkClientVerifier` (`offer_client_auth`, `client_auth_mandatory`, `verify_client_cert`) |
| Peer key from `SSL_get0_peer_rpk` (32-byte raw public key) | `extract_ed25519_key` (44-byte SPKI or 32-byte key) |
| Certificate signature verified as Ed25519 over the transcript | `verify_ed25519_signature` via `PublicKey::verify_raw` |

## Crate mapping

- `crates/tonutils-adnl/src/adnl/quic_tls.rs` (private module): SNI
  derivation, key extraction, `TonServerCertVerifier`, `TonRpkClientVerifier`,
  `SniRpkResolver`, `make_quinn_client_config`, `make_quinn_server_config`.
- `crates/tonutils-adnl/src/adnl/quic.rs`: `QuicSession` (connect, query,
  message, DHT/overlay helpers) and `QuicServer` (bind, accept, reported
  SNI), plus loopback and negative-handshake tests.
- `crates/tonutils-adnl/src/adnl/crypto.rs`: `sign_raw`/`verify_raw`
  (standard Ed25519) and `KeyPair::private_key_der`, which embeds the
  original 32-byte seed so `rustls` derives exactly the public key the pair
  carries.
- `crates/tonutils-mempool/src/quic_session.rs` and `endpoints.rs`: dial
  targets for overlay members (see [overlay.md](overlay.md)).

## Evidence

- Unit tests in `quic_tls.rs`: the SNI fixed vector above, both certificate
  encodings plus foreign encodings (wrong length, foreign SPKI prefix,
  non-point y coordinate, X.509 container), verifier pinning and
  `requires_raw_public_keys`/`offer_client_auth`/`client_auth_mandatory`
  flags, standard Ed25519 `CertificateVerify` acceptance with tampering,
  wrong-scheme, wrong-key, and truncation rejection, and a binding check
  that `rustls` signs with the pair's own identity key.
- Loopback tests in `quic.rs`: query/answer exchange with the reported
  short-id SNI, `InvalidQuicAnswer` fail-closed behavior, and three
  negative handshakes (mismatched server key, client without a raw public
  key, unrecognized SNI).
- Cross-SDK interop (local, 2026-10-12) against upstream's
  `test/quic-interop-rust` reference pair with its ALPN patched from
  `hq-interop` to `ton`: our `QuicSession` completed the handshake against
  `rpk-server` and received an HTTP 200, and upstream's `rpk-client`
  completed our mandatory-client-auth, SNI-gated handshake and finished all
  of its phases (path migration, CID rotation) with exit code 0.
- Live reachability: `mainnet_overlay_quic_discovery_returns_peers` in
  `tonutils-mempool` adds a raw Version Negotiation phase whose
  `vn_replies=` line separates firewalled targets from rejected handshakes;
  the CI run of that test is the pending evidence.

## Missing work

- A quictls/C++ client against our mandatory-client-auth server is the one
  pairing upstream's `test-quic-interop.sh` matrix never exercised (their
  Test 3 rustls server ran with `offer_client_auth() = false`); tracked in
  `TODO.md`.
- Whether mainnet full nodes answer QUIC from third-party sources is a
  reachability question, answered only by the CI live test.
- Concurrent routing and unsolicited-message handling on `QuicSession` are
  not implemented (see the [protocol matrix](protocol-matrix.md)).
