# Network protocol matrix

This page records the protocol surface currently implemented and tested in the
repository. It distinguishes checked local behavior from live interoperability
that still requires an official-node fixture or a configured CI peer.

| Surface | Checked wire facts | Local evidence | Current limit |
| --- | --- | --- | --- |
| ADNL TCP | 32-byte receiver address; 32-byte sender key; 32-byte AES-parameter hash; 160-byte encrypted parameters. Encrypted frames use a little-endian length, 32-byte nonce, payload, and SHA-256 integrity hash. | `tonutils-adnl/src/adnl/primitives/handshake.rs`, `codec.rs`, and loopback/negative tests. | No official-node handshake fixture is checked in. |
| ADNL UDP direct | Destination ADNL id prefixes the encrypted packet. `adnl.packetContents` carries optional flags, sequence/confirmation values, reinit fields, and signature. | `tonutils-adnl/src/adnl/udp.rs` and `udp_tests.rs`. Wrong destination, invalid integrity, malformed packets, and replayed sequence numbers are dropped or rejected. | `adnl.message.part` fragmentation is not implemented. |
| ADNL UDP channel | Channel packets use directional AES state derived from both channel public keys and peer ids; channel create/confirm is validated before switching packet ids. | `tonutils-adnl/src/adnl/udp.rs` channel tests. | Official packet captures and NAT/address-list interoperability are still missing. |
| Overlay broadcast | `overlay.message` prefixes the overlay id. `tonNode.externalMessageBroadcast` carries a serialized external BoC. `overlay.broadcastFec` carries FEC metadata and a serialized RaptorQ `EncodingPacket` (payload id plus symbol bytes). | `tonutils-mempool/src/udp_session.rs`, `quic_session.rs`, and seed-scanner tests. Hash, source block, symbol bounds, metadata consistency, and reconstructed payload type are checked. | FEC signatures/certificates and official live captures are not yet verified. |
| RLDP/RLDP2 | No typed RLDP client/data path is currently exposed by this workspace. | Schema inventory only. | Add feature-gated RLDP2 transfer state and checked fixtures before claiming support. |
| ADNL QUIC | TLS ALPN is `ton`; SNI is `<first 32 hex>.<last 32 hex>.adnl`; the certificate SAN encodes the Ed25519 identity key. Queries and answers use checked `quic.query`/`quic.answer` constructors and matching ids. | `tonutils-adnl/src/adnl/quic.rs` unit and loopback tests. | Concurrent routing and unsolicited-message handling are not implemented; QUIC is not an unverified UDP fallback. |

## Failure semantics

Malformed, wrong-peer, tampered, stale, or replayed UDP packets are ignored by
the receive loop while the session remains usable. Local socket/resource errors
and repeated decode failures can terminate a session. QUIC query responses fail
closed when the answer is malformed or its id does not match the request; raw
bytes are not returned as a successful answer.

## Live mempool evidence

The ignored `configured_seed_delivers_valid_external_message` test keeps a
strict 30-second default delivery deadline. It validates every configured
`KEY@IP:PORT` seed, reports the seed/start/connected/delivery phases, and fails
on missing seeds, failed startup, zero connected peers, timeout, or a payload
that is not a valid TL-B `ExternalIn` message. Direct UDP remains the primary
live overlay transport.
