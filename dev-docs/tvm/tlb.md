# TL-B Data Models

## Purpose And Scope

TL-B defines TON's bit-level schema language for data stored in TVM cells. It is
used for blocks, messages, accounts, transactions, config objects, smart-contract
state, and many message body layouts. TL-B is separate from TL: TL serializes
byte-level protocol messages for ADNL, LiteAPI, DHT, overlays, and related
network APIs, while TL-B serializes structured values into cell bits and cell
references.

This document fixes the crate direction for TL-B runtime traits, model codecs,
and future macro support. It is a design record only: the first implementation
step should add small hand-written codecs and tests before introducing a
proc-macro crate or any heavy dependency.

## TL Compared With TL-B

| TL | TL-B |
| --- | --- |
| byte-level protocol serialization | bit-level cell serialization |
| used for ADNL, LiteAPI, DHT, overlays | used for blocks, messages, accounts, state |
| constructor ids are 32-bit little-endian ids | constructors are fixed bit tags or implicit |
| vectors and bytes are common primitives | refs, bits, `Maybe`, `Either`, `HashmapE` are common primitives |
| schema terms map to byte readers/writers | schema terms map to `Builder`, `Slice`, and child cells |

## Wire Format And Data Model

TL-B values are encoded as a sequence of bits in the current cell plus zero or
more references to child cells. A cell can contain at most 1023 data bits and at
most 4 direct references. Multi-cell structures must preserve the exact
placement required by the schema: storing a value inline and storing it behind a
reference are different encodings even if the child value is otherwise identical.

The initial Rust mapping should use `src/tvm/builder.rs`, `src/tvm/slice.rs`,
`src/tvm/cell.rs`, and `src/tvm/dict.rs` as the low-level runtime. Model code
should not build BoC bytes directly. The intended flow is:

1. `TlbSerialize` writes a value into a mutable `Builder`.
2. `Builder::build` produces a `Cell`.
3. `Slice` reads a value from a cell for `TlbDeserialize`.
4. BoC serialization remains a separate outer step.

The public TL-B module should eventually own model-level traits, error types,
and built-in schemas. Low-level bit, reference, integer, and dictionary
operations should stay in `tvm`.

## Runtime Trait Shape

The intended minimal traits are:

```rust
pub trait TlbSerialize {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()>;

    fn to_cell(&self) -> Result<Arc<Cell>> {
        let mut builder = Builder::new();
        self.store_tlb(&mut builder)?;
        Ok(builder.build()?)
    }
}

pub trait TlbDeserialize: Sized {
    fn load_tlb(slice: &mut Slice) -> Result<Self>;

    fn from_cell(cell: Arc<Cell>) -> Result<Self> {
        let mut slice = Slice::new(cell);
        let value = Self::load_tlb(&mut slice)?;
        ensure_empty(&slice)?;
        Ok(value)
    }
}
```

The traits should remain object-agnostic and should not require allocation for
simple values. `to_cell` and `from_cell` are convenience wrappers around the
low-level builder and slice APIs, not the canonical implementation points.
`TlbScheme` currently holds descriptive constructor metadata: constructor name,
result name, and optional static tag bits. Runtime codecs must stay usable
without generated schema metadata.

The first implementation should prefer hand-written implementations for a small
set of core models. Derive macros can be added later after the codec rules are
proven by fixtures.

## Struct Mapping

A TL-B product constructor maps to a Rust struct. Fields are encoded in schema
order and decoded in the same order. Field names in Rust should use normal snake
case even when the source schema uses protocol-specific names.

Example shape:

```text
int_msg_info$0 ihr_disabled:Bool bounce:Bool bounced:Bool
  src:MsgAddressInt dest:MsgAddressInt value:CurrencyCollection
  ihr_fee:Grams fwd_fee:Grams created_lt:uint64 created_at:uint32
  = CommonMsgInfo;
```

The Rust model should encode the constructor tag first, then each field. Boolean
fields store one bit. Fixed-width unsigned integers use the exact bit width from
the schema and are encoded MSB-first. Signed integers use the TVM two's-complement
integer APIs for the declared width. Big integer fields must use the explicit
large integer builder and slice APIs when the width exceeds `u64`.

No implementation should reorder fields for Rust convenience. No implementation
should silently accept extra bits or refs for top-level exact decodes unless the
caller explicitly asks for prefix decoding.

## Enum And Constructor Mapping

A TL-B sum type maps to a Rust enum. Each variant corresponds to one constructor
that returns the same TL-B result type. Decoding reads enough bits to distinguish
the constructor, then delegates to the variant fields. Encoding writes the
variant's constructor tag and fields.

Constructor handling must support these cases:

- Fixed bit tags such as `$0`, `$10`, or `$110`.
- Hex tags such as `#_`, `#3`, or longer schema tags when present in upstream
  definitions.
- Implicit constructors, where the schema has no serialized tag and the context
  selects the only valid constructor.
- Parameterized constructors, where type-level parameters affect field widths or
  nested value codecs but are not necessarily serialized.

When variants have overlapping tag prefixes, decoding must use the complete
constructor set for the result type and choose only an unambiguous full tag. A
shorter prefix must not be accepted if a longer constructor could still match in
that context. The future macro implementation should generate a compact tag
decision tree and a validation test for ambiguous variants.

## Cell References

`^X` means the current cell stores a reference to a child cell that encodes `X`.
Serialization must build the child cell with `X::store_tlb`, then store it as a
reference in the parent builder. Deserialization must load one reference, create
a slice over that child cell, decode `X`, and require the child slice to be
consumed for exact referenced values.

Reference errors must distinguish:

- The current slice has no reference to load.
- The referenced cell exists but cannot be parsed as `X`.
- The referenced cell parses as `X` but has trailing bits or references where the
  schema requires exact consumption.
- The parent builder would exceed the 4-reference cell limit.

Inline `X` and referenced `^X` should use the same model type when possible, but
the wrapper position decides whether the codec operates on the current builder
or a child builder.

## Maybe And Either

`Maybe X` stores a one-bit presence marker:

- `0`: no value follows.
- `1`: `X` follows according to its own mapping.

`Maybe X` should map to `Option<T>` for owned values. `Option<Cell>` is valid
when the schema intentionally stores raw cells, but model types should use
`Option<T>` for typed data.

`Either X Y` stores a one-bit branch marker:

- `0`: left branch `X`.
- `1`: right branch `Y`.

`Either X Y` should map to an enum, not to two optional fields. For common
inline-or-reference patterns such as `Either X ^X`, the crate may provide a
small helper enum if it improves readability, but generated model code should
still make branch choice explicit.

## VarUInteger

`VarUInteger n` stores a length prefix followed by an unsigned integer payload.
The prefix width is `ceil(log2(n))` bits and the prefix value is the number of
payload bytes. The maximum payload length is `n - 1` bytes. A zero prefix encodes
zero and stores no payload bytes.

`VarUInteger 16`, used by `Grams`, has a 4-bit length prefix and can encode up
to 15 payload bytes. The integer payload is big-endian and must be canonical:
non-zero values must use the shortest possible byte length, and zero must use
length 0. Deserialization should reject overlong encodings such as length 2 for
a value that fits in one byte, and length greater than the schema maximum.

The Rust mapping should use `u64` only when the schema maximum is known to fit.
Currency values and generic `VarUInteger n` should use the crate's big unsigned
integer APIs or a small wrapper that preserves canonical length constraints.

## HashmapE

`HashmapE n X` stores an optional Patricia-tree dictionary:

- `0`: empty dictionary.
- `1`: followed by one reference to a `Hashmap n X` root edge.

Keys are fixed-width MSB-first bitstrings. Edges store compressed `HmLabel`
prefixes, and fork children are references selected by the next key bit.

`src/tvm/dict.rs` implements the current generic dictionary foundation:

- `BitKey` stores fixed-width key bits with canonical zeroed unused final-byte
  bits.
- `HashmapE<V>` stores sorted `BitKey` entries and serializes them as canonical
  TL-B `HashmapE`.
- `Builder::store_hashmap_e_with` and `Slice::load_hashmap_e_with` encode and
  decode values through callbacks, leaving concrete `X` codecs to callers.

Serialization emits canonical `HmLabel` forms (`hml_short`, `hml_long`, or
`hml_same`). Deserialization accepts all valid label forms and rejects overlong
labels, missing fork references, duplicate decoded keys, and key-width
mismatches. Typed TL-B models should wrap these callback APIs rather than
duplicating dictionary traversal.

## Tag And Exact Decode Errors

TL-B decoding errors should be precise enough for fixture debugging and proof
verification. The intended `TlbError` cases include:

- Constructor tag mismatch, including the expected constructor or result type and
  the bits actually observed when available.
- Slice underflow for missing bits.
- Reference underflow for missing child cells.
- Builder overflow for more than 1023 bits or more than 4 references.
- Invalid reference payload for `^X`.
- Non-canonical integer, `VarUInteger`, or dictionary encoding.
- Unsupported exotic or proof-sensitive cell shape when a model requires an
  ordinary cell.
- Trailing bits or references after an exact decode.

The low-level `tvm` errors should remain reusable. `TlbError` can wrap them, but
model code should add schema context before returning an error where practical.

## Macro Direction

Macro support is a later Phase 1 implementation step. This slice does not create
a proc-macro crate and does not add dependencies.

The eventual macro design should support two complementary forms:

- A derive macro for manually written Rust structs and enums, where attributes
  provide constructor tags, field widths, references, and helper codecs.
- A schema macro or code generator that consumes checked upstream TL-B snippets
  and emits deterministic Rust models plus tests for constructor tags and field
  order.

The derive form is useful for stable public model names and hand-curated APIs.
The schema-driven form is useful for broad upstream coverage and drift checks.
Both forms must generate code that calls the same `TlbSerialize` and
`TlbDeserialize` traits, so hand-written and generated models compose.

Proc-macro support should be an explicit workspace and dependency decision. If a
new crate is added, keep it feature-gated so users who only need low-level TVM
primitives do not pay macro compile cost.

## Current Crate Mapping

Current implemented building blocks:

- `src/tvm/cell.rs`: cell representation, hash/depth behavior, refs, and BoC
  integration.
- `src/tvm/builder.rs`: bit, integer, reference, and dictionary storage helpers.
- `src/tvm/slice.rs`: bit, integer, reference, and dictionary loading helpers.
- `src/tvm/dict.rs`: canonical `HashmapE` foundation.
- `src/tlb/mod.rs`: minimal public TL-B runtime with `TlbSerialize`,
  `TlbDeserialize`, `TlbScheme`, `TlbError`, fixed-tag helpers, exact decode
  checks, `Maybe`, `Either`, referenced value helpers, and canonical
  `VarUInteger` helpers.

The crate does not yet expose derive macros, schema parsing, or built-in TL-B
blockchain models.

## Needed TON Models

Core models to implement first:

- `CommonMsgInfo`, including internal, external-in, and external-out message
  info.
- `Message` and typed body/state-init reference handling.
- `StateInit`.
- `Account` and `AccountState`.
- `ShardAccount`.
- `Transaction` and transaction phases.
- Block header, value flow, extra data, and shard hashes.
- Config parameters needed by LiteClient and contract workflows.

Each model should cite the upstream TL-B schema revision used for its
constructor definitions and should have fixture-backed roundtrip tests before it
is used by high-level contract APIs.

## Testing Strategy

TL-B tests should be layered:

- Unit tests for constructor tag matching, `Maybe`, `Either`, references,
  `VarUInteger`, and dictionary callbacks.
- Golden BoC fixture tests for known blocks, messages, accounts, config objects,
  and proofs.
- Roundtrip tests that decode a fixture, re-encode it, and compare cell hashes
  rather than relying only on byte-for-byte BoC equality.
- Negative tests for invalid tags, slice underflow, missing references,
  non-canonical `VarUInteger`, duplicate dictionary keys, and trailing data.
- Cross-checks against upstream TON tools, tonutils-go, tongo, or pytoniq-core
  behavior where the protocol leaves room for implementation mistakes.

## Invariants And Edge Cases

- Respect the 1023-bit and 4-reference cell limits at every encoding step.
- Treat bit widths as schema facts, not Rust type hints.
- Preserve inline versus referenced placement exactly.
- Reject non-canonical encodings for values where TON defines a canonical form.
- Do not accept ambiguous constructor tags.
- Require exact consumption for complete model decodes and referenced child
  values.
- Keep generated or macro-expanded code deterministic and formatted.
- Keep TL-B schema source tracking in `dev-docs/operations/source-tracking.md`
  or in the model-specific documentation added with each schema family.

## Missing Work

- Decide whether derive support lives in a separate proc-macro crate and which
  feature gate exposes it.
- Implement the first hand-written core TL-B models and fixture tests.
- Add schema drift checks against upstream `ton-blockchain/ton` TL-B sources.
- Add public rustdoc examples once the first models exist.
