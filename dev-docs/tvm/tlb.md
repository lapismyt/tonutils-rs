# TL-B Data Models

## Purpose And Scope

TL-B defines TON's bit-level schema language for data stored in TVM cells. It is
used for blocks, messages, accounts, transactions, config objects, smart-contract
state, and many message body layouts. TL-B is separate from TL: TL serializes
byte-level protocol messages for ADNL, LiteAPI, DHT, overlays, and related
network APIs, while TL-B serializes structured values into cell bits and cell
references.

This document fixes the crate direction for TL-B runtime traits, model codecs,
schema parsing, and macro support. The current implementation includes
hand-written codecs for the first blockchain model surface, raw-preserving
wrappers for deeper block families, a deterministic schema parser and
checked-summary workflow, and an optional proc-macro crate behind
`tlb-derive`.

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

Hand-written implementations remain preferred for protocol-critical built-in
models until schema generation is complete. The derive macro is available for
application-specific TL-B bodies and small local schemas; it emits the same
`TlbSerialize` and `TlbDeserialize` traits used by handwritten models.

## Derive Macro Architecture

`tonutils-macros` is a workspace proc-macro crate enabled only through the
`tlb-derive` feature. The main crate re-exports `tlb::Tlb` and `tlb::TlbDerive`
under that feature. Default builds do not depend on `syn`, `quote`, or
`proc-macro2` through this path.

Supported derive attributes:

- `#[tlb(tag = "0101")]`, `#[tlb(tag = "0b0101")]`,
  `#[tlb(tag = "0x5")]`, and `#[tlb(tag = "#5")]` write and check fixed
  constructor tags. Hex forms expand to four bits per digit.
- `#[tlb(bits = N)]` stores exact-width primitive integers and `[u8; 32]`
  through `StoreBits<N>` and `LoadBits<N>`. Unsigned primitive fields `u8`,
  `u16`, `u32`, `u64`, and `u128` infer their natural bit width when this
  attribute is omitted. Signed integer fields require an explicit `bits`
  attribute. Float primitive fields are rejected because the runtime does not
  define TL-B float semantics.
- `#[tlb(reference)]` or `#[tlb(ref)]` stores a field as `^T` using
  `store_ref_tlb` and `load_ref_tlb`.

The macro currently supports structs and tagged enums. Enum variants must have
explicit tags. `TlbDeserialize::from_cell` remains the exact decode
entry point and rejects trailing bits or references after the generated decoder
returns.

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

## HashmapAug And HashmapAugE

`HashmapAug n X Y` uses the same fixed-width Patricia-tree key layout as
`Hashmap n X`, but every leaf and fork stores an augmentation value of type `Y`:

- `ahm_edge#_ label:(HmLabel ~l n) ... = HashmapAug n X Y`;
- `ahmn_leaf#_ extra:Y value:X = HashmapAugNode 0 X Y`;
- `ahmn_fork#_ left:^(HashmapAug n X Y) right:^(HashmapAug n X Y) extra:Y`;
- `ahme_empty$0 extra:Y = HashmapAugE n X Y`;
- `ahme_root$1 root:^(HashmapAug n X Y) extra:Y = HashmapAugE n X Y`.

`src/tvm/dict.rs` provides `HashmapAug<V, E>` for non-empty dictionaries and
`HashmapAugE<V, E>` for optional-root dictionaries with a top-level extra.
Decoded leaf, fork, and top-level augmentation values are preserved. Canonical
construction from entries is available for tests and SDK-created values, but it
requires the caller to provide augmentation values because TON aggregation rules
are schema-specific.

The callback APIs mirror `HashmapE`: `Builder::store_hashmap_aug_with`,
`Builder::store_hashmap_aug_e_with`, `Slice::load_hashmap_aug_with`, and
`Slice::load_hashmap_aug_e_with`. Concrete models supply both value and
augmentation codecs.

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

## Schema And Macro Direction

Phase 1 macro/schema support is the checked schema workflow in
`src/tlb/schema.rs`:

- users can parse TL-B text with `parse_schema`,
- inspect constructor names, explicit tags, grouped references, field text, and
  result types,
- generate a deterministic checked summary with `generate_summary`,
- compare generated output against a checked-in snapshot for drift detection.

`examples/tlb_schema_codegen.rs` demonstrates this path with a small
user-defined schema snippet and separately verifies the built-in
`BLOCK_PHASE1_TLB` summary. The built-in Phase 1 block/config/proof schema
slice remains in `src/tlb/schemas/block_phase1.tlb`; its checked summary remains
in `src/tlb/generated/block_phase1.rs`.

The checked schema workflow remains the broad upstream-schema path. A separate
optional `tonutils-macros` proc-macro crate now covers hand-written Rust
structs and enums without adding compile cost for users who do not enable the
`tlb-derive` feature.

Macro work supports two complementary forms:

- A derive macro for manually written Rust structs and enums, where attributes
  provide constructor tags, field widths, references, and helper codecs.
- A schema macro or code generator that consumes checked upstream TL-B snippets
  and emits deterministic Rust models plus tests for constructor tags and field
  order.

The derive form is useful for stable public model names and hand-curated APIs.
The schema-driven form is useful for broad upstream coverage and drift checks.
Both forms must generate code that calls the same `TlbSerialize` and
`TlbDeserialize` traits, so hand-written and generated models compose.

Keep proc-macro support feature-gated so users who only need low-level TVM
primitives do not pay macro compile cost.

## Current Crate Mapping

Current implemented building blocks:

- `src/tvm/cell.rs`: cell representation, hash/depth behavior, refs, and BoC
  integration.
- `src/tvm/builder.rs`: bit, integer, reference, and dictionary storage helpers.
- `src/tvm/slice.rs`: bit, integer, reference, and dictionary loading helpers.
- `src/tvm/dict.rs`: canonical `HashmapE` and augmentation-preserving
  `HashmapAug`/`HashmapAugE` foundation.
- `src/tlb/mod.rs`: public TL-B runtime with `TlbSerialize`,
  `TlbDeserialize`, `TlbScheme`, `TlbError`, fixed-tag helpers, exact decode
  checks, `Maybe`, `Either`, referenced value helpers, and canonical
  `VarUInteger` helpers.
- `tonutils-macros/`: optional proc-macro crate for deriving the TL-B
  runtime traits on hand-written Rust structs and enums.

The first built-in hand-written blockchain model slice is implemented in
`src/tlb/message.rs` from the current upstream
`ton-blockchain/ton` `crypto/block/block.tlb` message definitions
(`https://github.com/ton-blockchain/ton/blob/master/crypto/block/block.tlb`).
It covers:

- `Anycast` with `depth:(#<= 30)` encoded in five bits and constrained to
  `1..=30`.
- `MsgAddressInt` constructors `addr_std$10` and `addr_var$11`.
- `MsgAddressExt` constructors `addr_none$00` and `addr_extern$01`.
- `MsgAddress` as an anonymous wrapper over `MsgAddressInt` or
  `MsgAddressExt`, without an additional tag.
- `Grams` as canonical `VarUInteger 16`.
- `CurrencyCollection` with `HashmapE 32 (VarUInteger 32)` extra currencies.
- `TickTock`.
- current upstream `StateInit` with
  `fixed_prefix_length:(Maybe (## 5))`, `special:(Maybe TickTock)`, and
  `code`, `data`, `library` as `Maybe ^Cell`.
- `SimpleLib` and current upstream `StateInitWithLibs`, using
  `library:(HashmapE 256 SimpleLib)`.
- `CommonMsgInfo` constructors `int_msg_info$0`, `ext_in_msg_info$10`, and
  `ext_out_msg_info$11`; the internal constructor uses current upstream
  `extra_flags:(VarUInteger 16)`.
- `CommonMsgInfoRelaxed` constructors `int_msg_info$0` and
  `ext_out_msg_info$11`; there is no external-in relaxed constructor, and
  relaxed internal `src` is `MsgAddress`.
- `Message Any` with explicit preservation of inline versus referenced
  `StateInit` and body cells.
- `MessageRelaxed Any` with the same init and body placement rules and
  `CommonMsgInfoRelaxed`.
- `LibRef` constructors `libref_hash$0` and `libref_ref$1`.
- The closed `OutAction` family: `action_send_msg#0ec3c86d`,
  `action_set_code#ad4de08e`, `action_reserve_currency#36e6b809`, and
  `action_change_library#26fa1dd4`.
- `OutList`, using upstream linked-list constructors
  `out_list_empty$_ = OutList 0` and
  `out_list$_ {n:#} prev:^(OutList n) action:OutAction = OutList (n + 1)`.
  The Rust model exposes `Vec<OutAction>` in execution/schema order: the first
  vector item is deepest next to `out_list_empty$_`, and the last item is stored
  in the root node. Encoding and decoding enforce the 255-action TON limit.
- `AccStatusChange` constructors `acst_unchanged$0`, `acst_frozen$10`, and
  `acst_deleted$11`.
- `StorageUsed` as `cells:(VarUInteger 7)` and `bits:(VarUInteger 7)`, with
  canonical variable-width integer payloads and a maximum payload length of six
  bytes per field.
- `TrActionPhase` as the upstream implicit constructor `tr_phase_action$_`.
  It stores `success`, `valid`, `no_funds`, `status_change`, optional
  `total_fwd_fees` and `total_action_fees`, signed `result_code`, optional
  signed `result_arg`, four `uint16` action counters, `action_list_hash:bits256`,
  and `tot_msg_size:StorageUsed`.
- `src/tlb/transaction.rs` implements transaction-description phase models:
  `TrStoragePhase`, `TrCreditPhase`, `ComputeSkipReason`, `TrComputePhase`,
  `TrBouncePhase`, `SplitMergeInfo`, and the full `TransactionDescr`
  constructor family (`trans_ord$0000`, `trans_storage$0001`,
  `trans_tick_tock$001`, `trans_split_prepare$0100`,
  `trans_split_install$0101`, `trans_merge_prepare$0110`, and
  `trans_merge_install$0111`).
- `TransactionDescr.action:(Maybe ^TrActionPhase)` maps to
  `Option<TrActionPhase>` but preserves the referenced child-cell placement and
  exact child decode semantics.
- Account state models from upstream `block.tlb`: `StorageExtraInfo` with
  three-bit tags `storage_extra_none$000` and `storage_extra_info$001`,
  `StorageInfo` with upstream field order
  `used`, `storage_extra`, `last_paid`, `due_payment`, `AccountState`,
  `AccountStorage`, `AccountStatus`, `Account`, and `ShardAccount`.
- `DepthBalanceInfo` as
  `depth_balance$_ split_depth:(#<= 30) balance:CurrencyCollection`, enforcing
  the five-bit `0..=30` split-depth bound.
- `ShardAccounts` as
  `HashmapAugE 256 ShardAccount DepthBalanceInfo`.
- Concrete `HashUpdateAccount` for
  `update_hashes#72 old_hash:bits256 new_hash:bits256 = HASH_UPDATE Account`.
- Top-level `transaction$0111`, including the child reference that stores
  `in_msg:(Maybe ^(Message Any))` and
  `out_msgs:(HashmapE 15 ^(Message Any))`. Outbound message dictionary keys are
  validated as 15-bit keys, and inbound/outbound messages are exact referenced
  `Message Any` payloads.
- Split/merge install `prepare_transaction:^Transaction` fields map to
  `Box<Transaction>` to keep Rust layout finite while preserving exact
  referenced decoding.
- `AccountBlock` as
  `acc_trans#5 account_addr:bits256 transactions:(HashmapAug 64 ^Transaction CurrencyCollection) state_update:^(HASH_UPDATE Account)`.
  The transaction dictionary is non-empty by construction and referenced
  transactions are exact `Transaction` payloads.
- `ShardAccountBlocks` as
  `HashmapAugE 256 AccountBlock CurrencyCollection`.

The slice intentionally does not implement full block headers, value flow,
`BlockExtra`, shard-state models, config params, or schema-derived model
generation. `TrActionPhase` does not embed an `OutList`; it stores only the
256-bit `action_list_hash` produced from the action list. Golden fixture
coverage remains deferred.

## Needed TON Models

Core models to implement next:

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

- Expand `tlb-derive` with parameterized TL-B types, implicit or CRC tags if
  needed, ambiguous-prefix checks, and negative compile tests.
- Add broader captured live/upstream golden fixtures for block, shard-state,
  config-param, Merkle proof, and Merkle update models.
- Replace raw-preserving block, shard-state, config, and proof wrappers with
  generated or handwritten typed models where stable.
