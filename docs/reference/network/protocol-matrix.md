# Network protocol matrix

This page records the protocol surface currently implemented and tested in the
repository. It distinguishes checked local behavior from live interoperability
that still requires an official-node fixture or a configured CI peer.

| Surface | Checked wire facts | Local evidence | Current limit |
| --- | --- | --- | --- |
| ADNL TCP | 32-byte receiver address; 32-byte sender key; 32-byte AES-parameter hash; 160-byte encrypted parameters. Encrypted frames use a little-endian length, 32-byte nonce, payload, and SHA-256 integrity hash. | `tonutils-adnl/src/adnl/primitives/handshake.rs`, `codec.rs`, and loopback/negative tests. | No official-node handshake fixture is checked in. |
| ADNL UDP direct | Destination ADNL id prefixes the encrypted packet. `adnl.packetContents` carries optional flags, sequence/confirmation values, reinit fields, and signature. | `tonutils-adnl/src/adnl/udp/mod.rs` and `udp/session.rs`, tested in `udp/tests.rs`. Wrong destination, invalid integrity, malformed packets, and replayed sequence numbers are dropped or rejected. | `adnl.message.part` fragmentation is not implemented. |
| ADNL UDP channel | Channel packets use directional AES state derived from both channel public keys and peer ids; channel create/confirm is validated before switching packet ids. | `tonutils-adnl/src/adnl/udp/cipher.rs`, tested in `udp/tests.rs`. | Official packet captures and NAT/address-list interoperability are still missing. |
| Overlay broadcast | `overlay.message` prefixes the overlay id. `tonNode.externalMessageBroadcast` carries a serialized external BoC. `overlay.broadcast` simple frames are verified against `overlay.broadcast.toSign{sha256(overlay.broadcast.id{src, sha256(data), flags}), date}` with `src` zeroed under `BroadcastFlagAnySender` (1), then unwrapped from the nested `tonNode.externalMessageBroadcast` to the raw BoC. `overlay.broadcastFec` carries FEC metadata and a serialized RaptorQ `EncodingPacket` (payload id plus symbol bytes). | `tonutils-tl/src/tl/overlay.rs` (`payload_if_valid`, signed/AnySender/tampering tests), `tonutils-mempool/src/udp_session.rs`, `quic_session.rs`, and seed-scanner tests. Hash, source block, symbol bounds, metadata consistency, and reconstructed payload type are checked. | FEC signatures/certificates and official live captures are not yet verified. |
| RLDP/RLDP2 | No typed RLDP client/data path is currently exposed by this workspace. | Schema inventory only. | Add feature-gated RLDP2 transfer state and checked fixtures before claiming support. |
| ADNL QUIC | TLS 1.3 only with ALPN `ton`. SNI is the peer's ADNL short id - SHA-256 over the TL-tagged Ed25519 key (`pub.ed25519`, `0xc6b41348`) - as lowercase hex split at 32 characters: `<hex[0:32]>.<hex[32:64]>.adnl`, matching upstream `ServerIdentity::sni` (`quic/quic-common.h`); an unrecognized SNI fails the handshake (fatal `unrecognized_name` upstream, no-resolved-certificate fatal alert in our rustls path). Certificates are RFC 7250 raw public keys in both directions: a 44-byte Ed25519 SubjectPublicKeyInfo (RFC 8410) or a bare 32-byte key, with no X.509 or SAN fallback. The client pins the server certificate to the dialed key, the server requires a client raw public key (upstream requests a client certificate and rejects a missing peer key in `parse_peer_id`), and handshake signatures are standard RFC 8032 Ed25519 verified with `PublicKey::verify_raw`. Queries and answers use the upstream `ton_api.tl` framings (`quic.query data:bytes = quic.Request`, `quic.answer data:bytes = quic.Response`); correlation is per bidirectional stream, frames carry no query id. Discovery targets runtime-registered overlay members: an advertised `adnl.address.quic` entry wins, otherwise `adnl_udp_port + 1000` (`quic/quic-sender.h::NODE_PORT_OFFSET`); the config's `dht.static_nodes` are `dht-server` deployments with no QUIC listener and serve only as a logged empty-registry fallback. | `tonutils-adnl/src/adnl/quic_tls.rs` (fixed SNI vector, key extraction shapes, verifier pinning, Ed25519 `CertificateVerify` acceptance/tampering) and `quic.rs` (loopback query exchange plus negative handshakes: mismatched server key, client without a raw public key, unrecognized SNI); `tonutils-mempool/src/endpoints.rs` target-preference, dedup, and cap tests; `overlay_inbound.rs` membership-evidence tests. | Concurrent routing and unsolicited-message handling are not implemented; QUIC is not an unverified UDP fallback; X.509 client certificates are intentionally rejected (upstream parity); mainnet QUIC ports are currently silent to third-party sources from GitHub Actions - CI run 38237835993 (2026-10-12) measured `vn_replies=0/13` with both `1.1.1.1:443` and `8.8.8.8:443` controls answering and the same hosts answering ADNL UDP on their base ports, so the targets drop packets before any handshake (no listener or filtered datagrams) rather than rejecting this client's TLS. |

## Failure semantics

Malformed, wrong-peer, tampered, stale, or replayed UDP packets are ignored by
the receive loop while the session remains usable. Local socket/resource errors
and repeated decode failures can terminate a session. QUIC query responses fail
closed when the answer payload is malformed; frames carry no query id, so
answers are correlated by their bidirectional stream, not by an id field.

## Live mempool evidence

The ignored `configured_seed_delivers_valid_external_message` test keeps a
strict 30-second default delivery deadline. It validates every configured
`KEY@IP:PORT` seed, reports the seed/start/connected/delivery phases, and fails
on missing seeds, failed startup, zero connected peers, timeout, or a payload
that is not a valid TL-B `ExternalIn` message. Direct UDP remains the primary
live overlay transport.
