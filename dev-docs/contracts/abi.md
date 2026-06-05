# ABI Data Model

The ABI module is the foundation for typed contract calls. It defines Rust
structs and enums for describing contract methods, message handlers, events,
parameters, selectors, and TON/TVM-oriented value types, plus scalar runtime
value conversion to and from TVM stack entries.

The public module is `tonutils::abi` and the core model/codecs are available
behind the existing `tvm` Cargo feature because they depend on TVM cells,
slices, addresses, and stack values. JSON ABI loading is available behind the
narrower `abi-json` feature, which adds `serde_json` on top of `tvm`.

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
helpers are intentionally value-level only: they do not call contracts or
select network providers.

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
- `Map` maps to `TvmStackEntry::Cell` containing a local
  `HashmapE key_bits ^AbiValueCell` dictionary. Keys must be fixed-width
  `uintN` or `intN`; `key_bits` is inferred from `N` when omitted. Duplicate
  encoded keys are rejected, and decoded entries are returned in canonical
  dictionary key order.
- `Optional(None)` maps to `TvmStackEntry::Null`; present optional values map
  as their nested type.

`Unknown` returns an explicit unsupported conversion error. Map support is a
deterministic local ABI policy and is not yet upstream compatibility evidence.

`encode_get_method_inputs` validates that a function is a `GetMethod` with
either no selector or a `MethodId`, checks input arity, and converts each input
value into a TVM stack entry. `decode_get_method_outputs` applies the same
get-method selector checks to returned stack entries and decodes them in ABI
output order. Both helpers are local stack codecs; method-id routing and
network execution remain contract-wrapper responsibilities. `Contract` exposes
`run_abi_get_method` and `run_abi_get_method_latest` for this workflow: they
derive the method id from `MethodId` or the ABI function name, call the normal
typed get-method path, and return ABI output values.

## Message Body Mapping

`encode_message_body` and `decode_message_body` support ABI input values for
`InternalMessage` and `ExternalMessage` functions. `Opcode(u32)` selectors are
encoded as the first 32 bits of the body. `None` selectors have no prefix.
`GetMethod` functions and `MethodId` selectors are rejected for message bodies.

Current body mappings:

- `Int { bits }` and `Uint { bits }` encode inline with the declared width.
- `Bool` encodes as one inline bit.
- `Address` encodes inline as a standard `MsgAddressInt`.
- `Bytes` and `String` encode as referenced byte-aligned snake cells.
- `Cell` and `Slice` encode as referenced cells.
- `Tuple` encodes fields inline in declared order.
- `Map` stores a reference to the same local `HashmapE key_bits ^AbiValueCell`
  dictionary root used by stack conversion.
- `Optional` encodes a `Maybe` bit followed by the nested value when present.

Decoding is exact and rejects opcode mismatches or trailing bits/references.
`Array` and `Unknown` are intentionally unsupported for message bodies until a
sequence layout policy is documented.

`encode_payload_components` and `decode_payload_components` expose the same
component mapping without a selector prefix. `encode_event_payload` and
`decode_event_payload` apply that component mapping to `AbiEvent` fields:
`Opcode(u32)` selectors use the same 32-bit prefix as message bodies, `None`
has no prefix, and `MethodId` is rejected as get-method-only. This event
payload support is symmetric local-policy coverage only; checked bytes are not
yet claimed as upstream-captured event evidence.

## JSON Loader

`parse_abi_json_str` and `parse_abi_json_value` are compiled with
`abi-json`. The loader accepts a local schema with:

- top-level `name`, `version`, and `contracts`,
- contract `name`, optional `methods`, and optional `events`,
- function `name`, `kind`, optional `selector`, optional `inputs`, and optional
  `outputs`,
- event `name`, optional `selector`, and optional `fields`,
- parameter `name`, `type`, and optional boolean `optional`.

Function kinds use `get_method`, `internal_message`, or `external_message`
with short aliases `get`, `internal`, and `external`. Selectors are objects
with either `method_id` or `opcode`; numeric values may be JSON numbers,
decimal strings, or `0x` hex strings. A selector object containing both
`method_id` and `opcode` is rejected as ambiguous.

Types may be strings such as `uint64`, `int257`, `bool`, `bytes`, `string`,
`address`, `cell`, `slice`, `optional<uint32>`, or `array<cell>`. Recursive
object forms are also accepted:

- `{ "tuple": [parameter, ...] }`,
- `{ "array": type }`,
- `{ "optional": type }`,
- `{ "map": { "key": type, "value": type, "key_bits": optional_integer } }`,
- `{ "unknown": "raw-spelling" }`.

Map key types must be `uintN` or `intN`. When `key_bits` is omitted, it is
inferred from `N`; when provided, it must match the integer key width.

Diagnostics include JSON paths for missing fields, invalid JSON kinds,
ambiguous selectors, and known compatibility shapes that are not implemented
by the local loader yet. Local model validation still runs after parsing, so
integer-width and empty-name violations are reported through
`AbiJsonError::Model`.

## CLI ABI Invocation

The `cli` feature includes `abi-json`. `contract run-abi-get-method` loads an
ABI JSON file, selects a contract and get-method, parses repeated
`--arg name=json` values, encodes get-method inputs to a TVM stack, executes
the get-method through the selected liteserver, and decodes returned stack
entries into named ABI output values. If `--contract` is omitted, the ABI file
must contain exactly one contract. If `--method` is omitted, the selected
contract must contain exactly one get-method.

CLI argument parsing accepts JSON integer numbers or decimal/hex integer
strings for ints and uints, JSON booleans and strings, hex strings for bytes,
TON address strings, tuple objects keyed by ABI field name, arrays for stack
types where the ABI stack codec already supports arrays, optional `null`, and
BoC hex strings for cells and slices. Map arguments use arrays of
`{ "key": ..., "value": ... }` entries and are encoded with the local fixed
integer-key dictionary policy.

## Golden Fixtures

Checked ABI fixture metadata lives in `fixtures/abi/contracts.json`. The file
uses schema revision `1` with a top-level synthetic source note, capture date,
and a `fixtures` array. Each fixture stores evidence metadata
(`evidence_kind`, `source_url`, `source_commit`, `network`, `account`,
`block_id`, `method_id`, `capture_command`, and `compat_reference`), a local
ABI JSON document, input values in fixture-only JSON form, and the expected
offline wire artifacts:

- get-method fixtures store `input_stack_boc_hex`, `input_stack_root_hash`,
  returned `output_stack_boc_hex`, `output_stack_root_hash`, and expected
  decoded ABI outputs;
- message-body fixtures store `message_body_boc_hex`, the body root
  representation hash, and expected decoded inputs;
- map fixtures store ABI JSON with fixed integer keys and fixture-only
  `{ "key": ..., "value": ... }` entries for stack and message-body roundtrips.
- event fixtures store `message_body_boc_hex`, the event payload root
  representation hash, and expected decoded event fields generated from the
  local event payload policy.

The message-body BoCs are body-cell BoCs only. They are not full internal or
external message BoCs and do not include `CommonMsgInfo`, state init, fees, or
envelope metadata.

The current fixture set includes synthetic offline coverage generated from the
local ABI policy documented above and opt-in live capture templates for wallet
`seqno` and TEP-74 `get_wallet_address(owner)`. The synthetic entries lock
deterministic behavior for already implemented JSON loading, get-method stack
conversion, message-body encode/decode, and local map/dictionary conversion.
The opt-in entries are templates only until stack/result bytes from the capture
command are checked in, so independent compatibility validation remains open.

## Non-Goals

This step does not implement:

- independent upstream or live-captured ABI compatibility vectors.

The module should therefore not be described as ABI execution support. It is a
stable Rust vocabulary and scalar stack conversion foundation for follow-up
work.

## Next Steps

Planned follow-up work:

- cross-check checked ABI fixtures against accepted TON protocol evidence and
  compatibility references.
