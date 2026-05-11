# Wallet V4R2 And TON Mnemonics

This page records the first Wallet V4R2 and mnemonic implementation scope for
`src/wallet.rs`.

## Mnemonic Rules

TON mnemonic support uses:

- 24 English BIP-39 words.
- validation against the English word list.
- no-password seed-version check with PBKDF2-HMAC-SHA512, salt
  `TON seed version`, `390` iterations, first byte `0`.
- password seed-version check with PBKDF2-HMAC-SHA512, salt
  `TON fast seed version`, `1` iteration, first byte `1`.
- Ed25519 seed derivation with PBKDF2-HMAC-SHA512, salt `TON default seed`,
  `100000` iterations, and the first 32 bytes as the Ed25519 seed.
- PBKDF2 input password is `HMAC-SHA512(key = normalized mnemonic phrase,
  data = mnemonic password or empty)`. The key/data order and seed-version
  byte are checked against `pytoniq-core` compatibility fixtures.

The CLI never stores the mnemonic. It reads mnemonics from stdin, a file, or an
environment variable.

## Wallet V4R2

Persistent data:

- `seqno:(## 32)`
- `wallet_id:(## 32)`
- `public_key:(## 256)`
- `plugins:(HashmapE 256 int1)`

The external simple-send body is:

- `signature:bits512`
- `wallet_id:(## 32)`
- `valid_until:(## 32)`
- `seqno:(## 32)`
- `opcode:(## 32)`, currently `0`
- up to four `(mode:uint8, msg:^MessageRelaxed)` entries

The signature is Ed25519 over the representation hash of the cell after the
signature field.

## Code BoC Evidence

The embedded V4R2 code BoC is taken from `@ton/ton` `WalletContractV4` package
source. The embedded V5R1 code BoC is taken from `@ton/ton`
`WalletContractV5R1` package source. The checked tests pin the hashes produced
by this crate's cell decoder. A follow-up item tracks reconciling these package
hashes with the wallet hashes published in the current TON wallet-history docs.

The local BoC decoder currently verifies IEEE CRC32, while public TON code BoCs
commonly use the CRC32C BoC flag. Wallet code loading strips that BoC transport
checksum before cell decoding; the cell hash is independent of BoC transport
checksum bytes.
