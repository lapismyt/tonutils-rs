# ABI Data Model

The ABI module is the foundation for typed contract calls. It defines Rust
structs and enums for describing contract methods, message handlers, events,
parameters, selectors, and TON/TVM-oriented value types, plus scalar runtime
value conversion to and from TVM stack entries.

The public module is `tonutils::abi` and is available behind the existing
`tvm` Cargo feature. No separate ABI feature is introduced yet because later
encoding work will depend on TVM cells, slices, addresses, and stack values.

## Scope

The current model covers:

- ABI documents as `AbiDefinition { name, version, contracts }`.
- Contract entries as `AbiContract { name, methods, events }`.
- Functions as `AbiFunction { name, kind, selector, inputs, outputs }`.
- Events as `AbiEvent { name, selector, fields }`.
- Parameters as `AbiParameter { name, ty, optional }`.
- Selectors as `None`, `MethodId(u64)`, or `Opcode(u32)`.
- Function kinds for get-methods, internal messages, and external messages.
- Runtime values as `AbiValue` for integers, booleans, bytes, strings,
  addresses, cells, slices, tuples, arrays, and optional values.

Validation is intentionally limited to local invariants:

- required names must be non-empty after trimming whitespace,
- signed and unsigned integer widths must be in `1..=257`,
- tuple, array, map, and optional types are validated recursively,
- unknown type spellings must be non-empty so they can be preserved safely.

## Type Vocabulary

`AbiType` currently supports:

- `Int { bits }` and `Uint { bits }`,
- `Bool`,
- `Bytes`,
- `String`,
- `Address`,
- `Cell`,
- `Slice`,
- `Tuple(Vec<AbiParameter>)`,
- `Array(Box<AbiType>)`,
- `Map { key, value }`,
- `Optional(Box<AbiType>)`,
- `Unknown(String)`.

The `257` integer-width upper bound matches TVM integer capacity assumptions
used by TON stack values.

## Stack Value Mapping

`AbiValue::to_stack_entry`, `AbiValue::from_stack_entry`, `to_stack_entry`,
and `from_stack_entry` convert values against an explicit `AbiType`. These
helpers are intentionally value-level only: they do not load ABI JSON, select
functions, call contracts, or build message bodies.

Current stack mappings:

- `Int { bits }` and `Uint { bits }` map to `TvmStackEntry::Int`, with
  declared width validation and signed or unsigned range checks.
- `Bool` maps to TVM integer `-1` for true and `0` for false. Decoding rejects
  all other integer values.
- `Bytes` and `String` map to a `Cell` containing byte-aligned snake data.
  `String` decoding requires valid UTF-8.
- `Address` maps to a `Slice` containing canonical
  `MsgAddressInt::std(address)` bytes and decodes only standard internal
  addresses without anycast.
- `Cell` maps to `TvmStackEntry::Cell`.
- `Slice` maps to `TvmStackEntry::Slice`.
- `Tuple` maps to `TvmStackEntry::Tuple` and follows declared field order.
- `Array` maps to `TvmStackEntry::List`.
- `Optional(None)` maps to `TvmStackEntry::Null`; present optional values map
  as their nested type.

`Map` and `Unknown` return explicit unsupported conversion errors. Dictionary
ABI layout policy is still open and should not be inferred from these helpers.

## Non-Goals

This step does not implement:

- JSON ABI parsing or schema compatibility with Tongo or another external ABI
  format,
- message body construction,
- contract wrapper integration,
- CLI loading or invocation,
- golden ABI fixtures.

The module should therefore not be described as ABI execution support. It is a
stable Rust vocabulary and scalar stack conversion foundation for follow-up
work.

## Next Steps

Planned follow-up work:

- implement JSON parsing with precise diagnostics and schema tests,
- define message-body mappings for ABI values,
- define dictionary/map ABI codec policy,
- add ABI-driven get-method and message-body helpers to contract wrappers,
- wire ABI loading into CLI workflows,
- add golden fixtures and cross-checks against accepted TON protocol evidence
  and compatibility references.
