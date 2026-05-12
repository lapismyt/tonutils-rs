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

With `liteclient`, `WalletV4R2::send_external_message` is an accepted
submission adapter over `ContractProvider::send_external_message_boc`. It builds
the same signed external-in message BoC as the offline helper, includes
`StateInit` only when requested for deploy or first-message workflows, submits
exactly one BoC through the provider, and returns the provider's opaque
`liteServer.SendMsgStatus.status` value. Build failures, including the four
message action limit, happen before provider submission. Provider failures are
reported without treating BoC submission as transaction inclusion.

## Code BoC Evidence

The embedded V4R2 code BoC is taken from `@ton/ton` `WalletContractV4` package
source. The embedded V5R1 code BoC is taken from `@ton/ton`
`WalletContractV5R1` package source. The checked tests pin the hashes produced
by this crate's cell decoder.

As of 2026-05-12, the embedded wallet code hashes are reconciled with the
current TON wallet-history docs:

- V4R2:
  `feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0`.
- V5R1:
  `20834b7b72b112147e1b2fb457b84e74d1a30f04f737d4f62a668e9552d2b72f`.

`fixtures/wallets/state_init_addresses.json` records deterministic default
state-init/address fixtures. The V4R2 case uses wallet id `0x29a9a317`,
workchain `0`, and the same checked public key as the V5R1 fixtures. Tests load
the fixture JSON and compare the embedded code cell hash, serialized data cell
hash, `StateInit` cell hash, raw address, and non-bounceable URL-safe
user-friendly address.

The local BoC decoder currently verifies IEEE CRC32, while public TON code BoCs
commonly use the CRC32C BoC flag. Wallet code loading strips that BoC transport
checksum before cell decoding; the cell hash is independent of BoC transport
checksum bytes.

## Sources

- Official TON Wallets history page for V4R2 and V5R1 code hashes:
  <https://docs.ton.org/standard/wallets/history>.
- Official TON wallet interaction page for default V4R2 and V5 wallet ids:
  <https://docs.ton.org/standard/wallets/interact>.
