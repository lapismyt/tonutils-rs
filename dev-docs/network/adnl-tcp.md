# ADNL TCP

ADNL TCP is the first supported network transport in this crate. It is used for liteserver connections.

## Identity

Liteserver configs provide an Ed25519 public key. The client computes the server ADNL address by hashing the TL public key constructor id and key bytes.

## Handshake

Client sends a 256-byte packet:

| Range | Size | Field |
| --- | ---: | --- |
| `0..32` | 32 | receiver ADNL address |
| `32..64` | 32 | sender public key |
| `64..96` | 32 | hash of plaintext AES params |
| `96..256` | 160 | encrypted AES params |

Server decrypts AES params with a key derived from ECDH shared secret and the params hash. It responds with an encrypted empty packet.

## Frame Body

After handshake, every frame has encrypted:

- little-endian length,
- random 32-byte nonce,
- payload bytes,
- SHA-256 hash over nonce and payload.

The length excludes the 4-byte length field and includes nonce and hash.

## Limits

- Minimum encrypted body length: 64 bytes.
- Maximum encrypted body length: `1 << 24`.
- Maximum payload length: `(1 << 24) - 64`.

## Security Properties

ADNL TCP gives encryption and integrity for the session. It does not verify blockchain correctness. LiteAPI proof verification is a separate layer.

## Current Crate Mapping

- `src/adnl/primitives/handshake.rs`
- `src/adnl/primitives/codec.rs`
- `src/adnl/wrappers/peer.rs`

## Required Tests

- handshake success,
- handshake unknown receiver,
- codec roundtrip,
- partial frame,
- multi-frame buffer,
- too short frame,
- too long frame,
- tampered hash.
