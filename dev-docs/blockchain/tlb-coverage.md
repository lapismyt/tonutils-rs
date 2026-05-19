# Blockchain TL-B Coverage

## Purpose And Source

This page tracks the checked blockchain TL-B surface implemented in `src/tlb`.
The protocol source of truth is upstream `ton-blockchain/ton`
`crypto/block/block.tlb`; the local checked snapshot is
`src/tlb/schemas/block.tlb`.

The current snapshot is not yet the complete upstream file. It contains the
families that are backed by typed codecs or raw-preserving public wrappers. Full
upstream sync, constructor drift checks for every family, and fixture-backed
block/shard/config/proof roundtrips remain active TODO items.

## Coverage Matrix

| Upstream family | Rust model | Codec status | Tests or examples |
| --- | --- | --- | --- |
| `MsgAddress`, `Message`, `StateInit` | `tlb::message::*` | typed | TL-B unit tests, `tlb_message_roundtrip` |
| `Account`, `ShardAccount` | `tlb::transaction::*` | typed | TL-B unit tests, `tlb_account_state_roundtrip` |
| `Transaction`, phases, account blocks | `tlb::transaction::*` | typed | TL-B unit tests, `tlb_transaction_roundtrip`, `tlb_read_tx_data` |
| `ShardIdent`, `ExtBlkRef`, `BlockIdExt` | `tlb::block::*` | typed | block unit tests |
| `Block` | `tlb::Block` | typed root with referenced child cells | `tlb_block_wrapper_decode` |
| `BlockInfo`, `BlockPrevInfo` | `tlb::BlockInfo`, `tlb::BlockPrevInfo` | raw-preserving wrappers | schema summary check |
| `ValueFlow` | `tlb::ValueFlow` | constructor-checked raw payload | block unit tests |
| `BlockExtra`, `McBlockExtra` | `tlb::BlockExtra`, `tlb::McBlockExtra` | raw-preserving wrappers | schema summary check |
| `ShardState`, `ShardStateUnsplit` | `tlb::ShardState`, `tlb::ShardStateUnsplit` | constructor-checked raw payload | block unit tests |
| `ConfigParams` | `tlb::ConfigParams` | typed address, decoded `Hashmap 32 ^Cell` entries, raw-preserving wrappers for common param ids | `tlb_config_params_wrapper`, block unit tests |
| `HASH_UPDATE` | `tlb::HashUpdate` | typed | block unit tests |
| `MERKLE_PROOF`, `MERKLE_UPDATE` | `tlb::MerkleProof`, `tlb::MerkleUpdate` | exotic-cell wrappers with virtual hash checks | `proof_verify` |

## Derive And Adapter Surface

The optional `tlb-derive` feature enables the `tonutils-tlb-derive` proc-macro
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

The derive macro currently handles product structs and simple tagged enums. It
does not yet generate schema-driven dictionary adapters, parameterized TL-B
types, implicit CRC tags, ambiguous-prefix decision trees, or trybuild-style
negative tests. Those gaps are tracked in `TODO.md`.
