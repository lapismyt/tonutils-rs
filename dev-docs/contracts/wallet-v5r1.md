# Wallet V5R1

This page records the initial Wallet V5R1 implementation scope for
`tonutils-rs`. The public Rust surface lives in `src/wallet.rs` and is available
with the `tvm` feature.

## Scope

The first milestone is offline-safe and deterministic:

- `WalletV5R1Data` serializes and deserializes the persistent storage data.
- `WalletV5R1WalletId` packs and unpacks V5R1 wallet ids from a signed network
  global id and a V5R1 context.
- `WalletMessage` builds standard `action_send_msg` actions with
  `MessageRelaxed Any`.
- `WalletV5R1` builds `StateInit`, derives addresses, creates external signed
  request bodies, signs their cell hash with Ed25519, and wraps them in
  external-in message BoCs.

Live sending remains an adapter over `ContractProvider::send_external_message_boc`.
The wallet module must not hide provider failures or treat BoC submission as
proof that a transaction was accepted on chain.

## Wire Format

Official Wallet V5 documentation describes the persistent state as:

- `is_signature_allowed:(## 1)`
- `seqno:(## 32)`
- `wallet_id:(## 32)`
- `public_key:(## 256)`
- `extensions_dict:(HashmapE 256 int1)`

The implemented `WalletV5R1Data` maps this directly. The extensions dictionary
is decoded as `HashmapE<bool>` because `int1` is a single bit. The current public
builder creates an empty dictionary; non-empty extension management is follow-up
work.

The external signed request body uses opcode `0x7369676e` followed by:

- `wallet_id:(## 32)`
- `valid_until:(## 32)`
- `msg_seqno:(## 32)`
- `inner:W5InnerRequest`
- `signature:bits512`

The signature is Ed25519 over the representation hash of the signing cell that
contains the opcode, wallet id, timeout, seqno, and inner request, but not the
signature itself.

The initial `W5InnerRequest` support covers ordinary outbound actions only:

- `out_actions:(Maybe ^OutList)`
- `extended_actions:(Maybe W5ExtendedActionList)` is always serialized as absent
  and rejected when decoding.

This preserves the V5R1 255-action limit through the existing `OutList`
implementation.

## Wallet Id

Wallet V5R1 ids are stored as raw 32 bits:

```text
wallet_id = network_global_id XOR context_id
```

Client context is:

```text
context_id_client$1 wc:int8 wallet_version:uint8 counter:uint15
```

The default vectors covered by tests are:

- mainnet `network_global_id = -239`, workchain `0`, version `0`, subwallet `0`
  gives `0x7fffff11`.
- testnet `network_global_id = -3`, workchain `0`, version `0`, subwallet `0`
  gives `0x7ffffffd`.
- workchain `-1` with the same mainnet/testnet values gives `0x007fff11` and
  `0x007ffffd`.

Backoffice/custom context is preserved as a 31-bit value with leading bit `0`.

## Trust Assumptions

The wallet helper verifies only local serialization and local Ed25519 signature
construction. It does not verify deployed wallet code, account state, seqno
freshness, timeout acceptance, extension authorization, or transaction inclusion.

Address derivation is deterministic for the provided code cell and data cell.
This repo embeds a Wallet V5R1 code BoC from the `@ton/ton`
`WalletContractV5R1` package source and pins its decoded cell hash in tests.
Reconciling that package hash with the current TON wallet-history table remains
tracked follow-up work.

## Tests

`src/wallet.rs` contains offline tests for:

- Wallet V5R1 wallet-id default vectors and unpacking.
- Data cell roundtrip and empty extension dictionary encoding.
- Address stability from the same `StateInit`.
- Signed external body construction and signature verification.
- Rejection of more than 255 wallet messages.
- External inbound message BoC decoding.
- Embedded code BoC hash stability for the local decoder.

Missing fixture work is tracked in `TODO.md`.

## Sources

- Official TON Wallet V5 page for persistent state and default wallet-id
  semantics: <https://docs.ton.org/standard/wallets/v5>.
- Official TON Wallet V5 API page for TL-B message layout and signing flow:
  <https://docs.ton.org/standard/wallets/v5-api>.
- pytoniq/pytoniq-core and STON.fi ton-rs may be used as comparison evidence
  for behavior and API ergonomics, but implementation must stay native to this
  repository.
