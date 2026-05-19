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

`params` must encode TVM stack values in the format expected by liteserver.
The internal stack codec preserves nulls, integers, cells, slices, tuples,
lists, and explicit unsupported payload bytes. The checked offline fixture
`fixtures/tvm/stack.json` records deterministic non-empty input stack BoCs,
root hashes, decoded entry shapes, and canonical reserialization checks for
scalar, deep stack-chain, nested tuple/list, huge integer, cell/slice, and
unsupported payload cases. This confirms the crate's own offline format is
reproducible; successful live captures and cross-SDK comparisons are still
needed before claiming full TON node compatibility for every non-empty shape.

For opt-in live evidence, run the ignored
`live_non_empty_stack_run_get_method_smoke` test with
`TON_GLOBAL_CONFIG_JSON`, `TON_STACK_TEST_CONTRACT_ADDRESS`,
`TON_STACK_TEST_METHOD` defaulting to `seqno`, and `TON_STACK_TEST_JSON`.
`TON_STACK_TEST_ACCEPT_EXIT_CODE` may be set when the selected method is
expected to reject the supplied non-empty stack.
Successful `exit_code == 0` runs print fixture JSON with params/result BoCs and
decoded stack output. Non-zero accepted runs remain smoke tests only and should
not be promoted to captured compatibility fixtures.

The public conversion layer is:

- `ToTvmStack` and `FromTvmStack` for full get-method argument and result
  stacks,
- `ToTvmStackEntry` and `FromTvmStackEntry` for one stack item,
- `TvmStackConversionError` for arity, type, integer range, bool, and address
  failures.

Built-in conversions cover `()`, `TvmStack`, `Vec<TvmStackEntry>`,
`TvmStackEntry`, signed and unsigned Rust integers, `BigInt`, `BigUint`,
`bool`, standard internal `Address` stack slices, `Arc<Cell>` cells,
`Option<T>` as `Null` or the inner entry, and tuples up to eight fields.

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
- conversion-trait helpers `run_get_method_as`,
  `run_get_method_by_name_as`, `run_get_method_latest_as`, and
  `run_get_method_by_name_latest_as`,
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
- Rust value and TVM stack conversion failures are `StackConversion`,
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
  semantics, conversion trait arity/range/type errors, latest-block typed
  helper routing, and address-bound typed-client delegation.
- Derive macro tests must cover supported code source attributes, default and
  explicit workchains, and rejected ambiguous struct shapes.
- Live-network tests remain ignored until checked fixtures or opt-in network
  configuration are available.

## Missing Work

- Capture successful live non-empty stack fixtures.
- Compare non-empty stack serialization with tonutils-go and tonlib behavior.
- Expand result stack decoding against live liteserver fixtures.
- Add known contract fixtures.
- Add proof verification.
- Add wallet signing, deployment builders, and ABI-driven message bodies.
