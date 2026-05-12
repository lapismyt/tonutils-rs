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
- With `liteclient`, `WalletV5R1` provides get-method helpers for `seqno`,
  `get_wallet_id`, `get_public_key`, `is_signature_allowed`, and
  `get_extensions` over the existing `ContractProvider` trait.

Live sending is an accepted adapter over
`ContractProvider::send_external_message_boc`. The wallet module must not hide
provider failures or treat BoC submission as proof that a transaction was
accepted on chain.

## Wire Format

Official Wallet V5 documentation describes the persistent state as:

- `is_signature_allowed:(## 1)`
- `seqno:(## 32)`
- `wallet_id:(## 32)`
- `public_key:(## 256)`
- `extensions_dict:(HashmapE 256 int1)`

The implemented `WalletV5R1Data` maps this directly. `HashmapE 256 int1` is
decoded as `HashmapE<bool>` because `int1` is a single bit. The public
`WalletV5R1Extensions` wrapper enforces the 256-bit key width and provides
hash-first insert, remove, contains, and iteration helpers.

Extension dictionary keys are 256-bit account hashes only. Address helpers on
`WalletV5R1Extensions` are convenience APIs that use `Address::hash_part`; they
do not encode or compare the workchain. This matches the V5 state layout and is
different from full `MsgAddressInt` serialization used by extended management
actions.

With `liteclient`, `extensions_raw_onchain` remains raw-preserving and returns
the stack cell or slice payload as `Arc<Cell>`. `extensions_onchain` layers typed
decoding on top of that payload and rejects malformed `HashmapE 256 int1`
encoding without changing the raw helper.

The external signed request body uses opcode `0x7369676e` followed by:

- `wallet_id:(## 32)`
- `valid_until:(## 32)`
- `msg_seqno:(## 32)`
- `inner:W5InnerRequest`
- `signature:bits512`

The signature is Ed25519 over the representation hash of the signing cell that
contains the opcode, wallet id, timeout, seqno, and inner request, but not the
signature itself.

The implemented `W5InnerRequest` support covers ordinary outbound actions and
Wallet V5R1 extended management actions:

- `out_actions:(Maybe ^OutList)`
- `extended_actions:(Maybe W5ExtendedActionList)`

`WalletV5R1ExtendedAction` supports the current V5R1 constructors:

- `add_extension#02 addr:MsgAddressInt`
- `delete_extension#03 addr:MsgAddressInt`
- `set_signature_auth_allowed#04 allowed:Bool`

The ordinary message API still serializes `extended_actions` as absent. The
explicit `*_with_extended_actions` builders accept management actions and enforce
the V5R1 255-action limit across ordinary and extended actions together.

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
The V5R1 get-method helpers decode successful TVM stack values from the wallet
address derived locally, but they do not prove that the deployed code at that
address is the embedded wallet code.

`WalletV5R1::send_external_message` builds the external-in BoC with the same
offline builder, includes `StateInit` only when requested, submits exactly one
BoC through the provider, and returns the provider's opaque
`liteServer.SendMsgStatus.status` value. Build failures, including action-count
limits, happen before provider submission. Provider failures are reported
without retry or interpretation.

Address derivation is deterministic for the provided code cell and data cell.
This repo embeds a Wallet V5R1 code BoC from the `@ton/ton`
`WalletContractV5R1` package source and pins its decoded cell hash in tests.
As of 2026-05-12, that hash is reconciled with the current TON wallet-history
table: `20834b7b72b112147e1b2fb457b84e74d1a30f04f737d4f62a668e9552d2b72f`.

`fixtures/wallets/state_init_addresses.json` records deterministic V5R1
state-init/address fixtures for the default mainnet wallet id `0x7fffff11` and
default testnet wallet id `0x7ffffffd`, both in workchain `0`, using the same
checked public key. Tests load the fixture JSON and compare the embedded code
cell hash, serialized data cell hash, `StateInit` cell hash, raw address, and
non-bounceable URL-safe user-friendly address.

## Tests

`src/wallet.rs` contains offline tests for:

- Wallet V5R1 wallet-id default vectors and unpacking.
- Data cell roundtrip and empty extension dictionary encoding.
- Address stability from the same `StateInit`.
- Signed external body construction and signature verification.
- Extension dictionary insertion, duplicate replacement, removal, key-width
  rejection, and empty/non-empty roundtrip.
- Extended action tags, multi-action snake-list encoding, mixed ordinary and
  extended signed bodies, and total action-count limit enforcement.
- External inbound message BoC decoding.
- Live send/deploy provider acceptance: deploy mode carries `StateInit`,
  non-deploy mode omits it, successful and failed provider submissions receive
  exactly one external-in BoC, provider status is propagated opaquely, and build
  failures do not call the provider.
- Embedded code BoC hash stability for the local decoder.
- Fixture-backed V5R1 mainnet/testnet default code, data, state-init, raw
  address, and user-friendly address derivation.
- Mock-provider coverage for V5R1 get-method routing, typed integer/public-key
  decoding, signature-auth status decoding, raw extension payload preservation,
  typed extension dictionary decoding, malformed extension dictionary rejection,
  non-zero exit codes, missing or undecodable stacks, wrong stack entry types,
  and provider error propagation.

Fee estimation, message tracking, and post-send transaction lookup remain
tracked separately.

## Sources

- Official TON Wallet V5 page for persistent state and default wallet-id
  semantics: <https://docs.ton.org/standard/wallets/v5>.
- Official TON Wallet V5 API page for TL-B message layout and signing flow:
  <https://docs.ton.org/standard/wallets/v5-api>.
- Official TON Wallets history page for the V5R1 code hash:
  <https://docs.ton.org/standard/wallets/history>.
- Official TON wallet interaction page for default V5 wallet ids:
  <https://docs.ton.org/standard/wallets/interact>.
- pytoniq/pytoniq-core and STON.fi ton-rs may be used as comparison evidence
  for behavior and API ergonomics, but implementation must stay native to this
  repository.
