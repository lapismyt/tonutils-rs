# Cryptographic Primitives

TON protocols use several cryptographic primitives at different layers. This page records what the crate needs and where each primitive is used.

## Hashes

| Primitive | Output | Used for |
| --- | ---: | --- |
| SHA-256 | 32 bytes | cell representation hashes, ADNL addresses, ADNL packet hashes |
| SHA-512 | 64 bytes | Ed25519 signing internals |
| CRC16 | 2 bytes | user-friendly address checksum, get-method name id helper |
| CRC32 / CRC32C | 4 bytes | TL constructor ids, BoC checksums depending on context |

## Ed25519 Public Keys

Liteserver and ADNL identity commonly use Ed25519 public keys. The TL constructor for a public key is:

```tl
pub.ed25519 key:int256 = PublicKey;
```

The public key bytes are 32 bytes. The crate stores them as compressed Edwards-Y points when validation is needed.

## ADNL Address Hash

For Ed25519 keys, ADNL short address is:

```text
sha256(pub.ed25519_constructor_id_le || public_key_bytes)
```

Constructor id little-endian bytes are:

```text
c6 b4 13 48
```

## Signatures

ADNL and DHT signatures must cover the exact TL-serialized object expected by the schema. Signing raw bytes and signing TL objects are not interchangeable.

Implementation rule:

- expose `sign_raw` only for protocol paths that explicitly sign raw bytes,
- expose `sign_tl` for TL object signatures,
- tests must verify that raw and TL signatures are not accidentally treated as equivalent.

## Shared Secrets

ADNL TCP handshake derives a shared secret from local secret key material and remote public key material. The current implementation uses curve operations from pure Rust dependencies and then derives AES-CTR parameters for the session.

## AES-CTR

ADNL TCP frames are encrypted with AES-256-CTR. CTR mode is a stream cipher construction:

- encryption and decryption are the same keystream XOR operation,
- nonce/key reuse is dangerous,
- each direction must use the correct tx/rx key and nonce pair.

## Randomness

Cryptographic randomness is required for:

- local ephemeral ADNL keys,
- AES session params,
- ADNL frame nonce bytes,
- query ids where unpredictability is useful.

Tests should use deterministic values only when cryptographic unpredictability is not part of the behavior under test.

## Crate Mapping

- `src/adnl/crypto.rs`: key types, signing, verification, shared secret helpers.
- `src/adnl/helper_types.rs`: ADNL address and AES params.
- `src/crc/`: CRC helpers.
- `src/tvm/cell.rs`: SHA-256 representation hash.

## Missing Work

- Separate protocol-level signature coverage docs per DHT and overlay type.
- Add known-good signature fixtures from upstream implementations.
- Audit CRC32 vs CRC32C usage names and algorithms.
