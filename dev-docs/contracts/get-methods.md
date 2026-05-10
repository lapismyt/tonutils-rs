# Smart Contract Get-Methods

Get-methods are read-only TVM executions against account state.

## LiteAPI Function

```tl
liteServer.runSmcMethod mode:# id:tonNode.blockIdExt account:liteServer.accountId method_id:long params:bytes = liteServer.RunMethodResult;
```

## Method Id From Name

Common tooling maps names to ids:

```text
(crc16(method_name) & 0xffff) | 0x10000
```

## Input Stack

`params` must encode TVM stack values in the format expected by liteserver. This must be verified with real fixtures.

## Result

Important result fields:

- `exit_code`,
- optional `result`,
- optional proof fields,
- `shardblk` execution context.

Non-zero `exit_code` is a contract result, not a transport failure.

## High-Level API Design

The `contracts` module provides `Contract<'a, P>` over any `ContractProvider`.
`ContractProvider` is implemented for both `LiteClient` and `LiteBalancer`.
`Contract<'a, P>` remains the address-bound execution wrapper. The
`ContractBlueprint` trait models contracts whose address is derived from a
fixed code BoC and typed TL-B data:

- `data()` returns the typed `TlbSerialize` state data,
- `code_boc()` returns fixed code BoC bytes,
- `workchain()` defaults to `0`,
- `state_init()` decodes code, serializes data, and fills `StateInit.code` and
  `StateInit.data`,
- `address()` calls the shared `address_from_state_init` primitive,
- `bind()` creates a normal address-bound `Contract<'a, P>`.

The optional `contract-derive` feature re-exports `tonutils::Contract` as a
derive macro. The macro accepts `#[contract(code = ...)]`,
`#[contract(code_hex = "...")]`, or `#[contract(code_file = "...")]` and
rejects unnamed/unit structs, missing `data`, extra fields, and multiple code
sources.

The wrapper provides:

- account address,
- raw, decoded, and simple latest-block account-state fetch,
- active-account balance, code, data, and `StateInit` accessors,
- get-method execution by numeric method id,
- get-method execution by method name,
- typed result helpers through `RunMethodResultExt`,
- high-level typed get-method helpers that fail on non-zero `exit_code`,
- raw external message BoC submission,
- account transaction-history lookup,
- `StateInit` address derivation from the serialized cell hash,
- direct embedding of address-bound `Contract<'a, P>` in typed clients.

Proof verification mode is not implemented yet. The wrapper preserves LiteAPI
proof bytes in the response structures.

## Error Semantics

High-level helpers return `ContractError<P::Error>`:

- provider failures preserve the original LiteClient or LiteBalancer error,
- non-zero get-method exit codes are `NonZeroExitCode`,
- TVM stack or TL-B decode failures are `Decode`,
- active-state helpers return missing-state variants for none, uninit, frozen,
  or absent code/data.

`run_get_method` remains a raw LiteAPI wrapper and returns non-zero exit codes
in `RunMethodResult` without treating them as transport errors.

## Capability Acceptance

- LiteClient and LiteBalancer must expose the same `ContractProvider` behavior
  for account state, get-methods, raw external BoC submission, and account
  transactions.
- Contract wrapper tests must prove provider routing, state decoding, active
  account field extraction, missing-state errors, method-name mapping, stack
  decoding, non-zero exit handling, raw external BoC preservation, transaction
  routing, state-init address derivation, blueprint state/address/bind
  semantics, and address-bound typed-client delegation.
- Derive macro tests must cover supported code source attributes, default and
  explicit workchains, and rejected ambiguous struct shapes.
- Live-network tests remain ignored until checked fixtures or opt-in network
  configuration are available.

## Missing Work

- Verify stack serialization.
- Expand result stack decoding against live liteserver fixtures.
- Add known contract fixtures.
- Add proof verification.
- Add wallet signing, deployment builders, and ABI-driven message bodies.
