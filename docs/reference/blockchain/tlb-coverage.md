# Blockchain TL-B Coverage

## Purpose And Source

This page tracks the checked blockchain TL-B surface implemented in `src/tlb`.
The protocol source of truth is upstream `ton-blockchain/ton`
`crypto/block/block.tlb`; the local checked snapshot is
`src/tlb/schemas/block.tlb`.

The current snapshot is not yet the complete upstream file. It contains the
families that are backed by typed codecs or explicit raw-preserving boundaries.
The schema generator records source hashes, constructor counts, and deterministic
revision strings for every checked-in TL/TL-B source.

## Coverage Matrix

| Upstream family | Rust model | Codec status | Tests or examples |
| --- | --- | --- | --- |
| `MsgAddress`, `Message`, `StateInit` | `tlb::message::*` | typed | TL-B unit tests, `tlb_message_roundtrip` |
| `Account`, `ShardAccount` | `tlb::transaction::*` | typed | TL-B unit tests, `tlb_account_state_roundtrip` |
| `Transaction`, phases, account blocks | `tlb::transaction::*` | typed | TL-B unit tests, `tlb_transaction_roundtrip`, `tlb_read_tx_data` |
| `ShardIdent`, `ExtBlkRef`, `BlockIdExt` | `tlb::block::*` | typed | block unit tests |
| `Block` | `tlb::Block` | typed root with referenced child cells | `tlb_block_wrapper_decode` |
| `BlockInfo`, `BlockPrevInfo` | `tlb::BlockInfo`, `tlb::BlockPrevInfo` | typed fields and conditional branches | block unit and fixture tests |
| `ValueFlow` | `tlb::ValueFlow` | typed v1/v2 currency groups | block unit and fixture tests |
| `BlockExtra`, `McBlockExtra` | `tlb::BlockExtra`, `tlb::McBlockExtra` | typed boundary with raw child families | block fixture tests |
| `ShardState`, `ShardStateUnsplit` | `tlb::ShardState`, `tlb::ShardStateUnsplitData` | typed stable scalars, accounts, balances, and master reference; explicit raw nested cells | block unit and extended fixture tests |
| `ConfigParams` | `tlb::ConfigParams`, `tlb::ConfigParamPayload` | typed stable payloads for params 15, 17, 19, 20/21, and 24/25; raw-preserving nested families | block unit and extended fixture tests |
| `HASH_UPDATE` | `tlb::HashUpdate` | typed | block unit tests |
| `MERKLE_PROOF`, `MERKLE_UPDATE` | `tlb::MerkleProof`, `tlb::MerkleUpdate` | exotic-cell wrappers with virtual hash checks | `proof_verify` |

## Derive And Adapter Surface

The optional `tlb-derive` feature enables the `tonutils-macros` proc-macro
crate and re-exports `tlb::Tlb` and `tlb::TlbDerive`. The macro generates the
existing runtime traits, not a separate runtime. Supported attributes are:

- `#[tlb(tag = "101")]`, `#[tlb(tag = "0b101")]`,
  `#[tlb(tag = "0x5")]`, and `#[tlb(tag = "#5")]` on structs or enum
  variants for fixed constructor tags. Hex tags expand to four bits per digit.
- `#[tlb(bits = N)]` on integer/hash fields for exact-width encoding through
  `StoreBits<N>` and `LoadBits<N>`. Unsigned primitive fields `u8`, `u16`,
  `u32`, `u64`, and `u128` infer their natural width. Signed integer fields
  require `bits`; float primitive fields are rejected because the runtime does
  not define TL-B float semantics.
- `#[tlb(reference)]` or `#[tlb(ref)]` on fields for `^T` child-cell encoding.

Runtime helpers added for macro and handwritten codecs:

- `CellRef<T>` for referenced typed values.
- `RawCell` for intentionally opaque cell payloads.
- `VarUInteger<N>` for canonical variable-width unsigned integers.
- `TlbHashmapE<T, N>` for typed dictionary values using TL-B codecs.

Exact top-level decode continues to use `TlbDeserialize::from_cell`, which
rejects trailing bits and references. Referenced decode uses `load_ref_tlb` and
also requires exact child consumption.

## Known Limits

`ShardStateUnsplitData` now decodes the stable scalar fields, typed
`ShardAccounts`, balances, and optional `BlkMasterInfo` directly from the
`shard_state#9023afe2` layout. `OutMsgQueueInfo`, `LibDescr` dictionaries, and
`McStateExtra` remain explicit raw cell references because their nested models
are not yet fixture-backed. `ShardStateUnsplit` remains as a compatibility
wrapper and exposes this typed view through `decode_typed()`.

The extended fixture set is `fixtures/compatibility/block_tlb_extended.json`.
It records the pinned upstream revision, capture date, source description,
canonical BoC policy, root representation hash, and payload SHA-256 for block,
shard-state, config, Merkle proof, and Merkle update samples. These are
independent offline schema-derived vectors, not live liteserver captures.

`MerkleProof` and `MerkleUpdate` validate exotic constructor kind and reference
count, preserve the original cell, and expose virtual-hash consistency checks.
Those checks are structural only: they do not establish a trusted proof root
or provide trustless Merkle proof verification. The cryptographic verification
gap remains tracked in `TODO.md`.

The derive macro currently handles product structs and simple tagged enums. It
does not yet generate schema-driven dictionary adapters, parameterized TL-B
types, implicit CRC tags, ambiguous-prefix decision trees, or trybuild-style
negative tests. Those gaps are tracked in `TODO.md`.
