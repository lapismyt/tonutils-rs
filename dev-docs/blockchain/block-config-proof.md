# Block, Config, And Proof TL-B Slice

## Purpose And Scope

This page records the Phase 1 TL-B coverage for block, config, shard-state,
and Merkle proof primitives. The source of truth is upstream
`ton-blockchain/ton` `crypto/block/block.tlb`. The crate keeps a small
upstream-derived slice in `src/tlb/schemas/block_phase1.tlb` and a checked
generated summary in `src/tlb/generated/block_phase1.rs`.

## Wire Format And Data Model

Covered constructors include:

- `shard_ident$00 shard_pfx_bits:(#<= 60) workchain_id:int32 shard_prefix:uint64`.
- `ext_blk_ref$_ end_lt:uint64 seq_no:uint32 root_hash:bits256 file_hash:bits256`.
- `block_id_ext$_ shard_id:ShardIdent seq_no:uint32 root_hash:bits256 file_hash:bits256`.
- `block#11ef55aa global_id:int32 info:^BlockInfo value_flow:^ValueFlow state_update:^(MERKLE_UPDATE ShardState) extra:^BlockExtra`.
- `value_flow#b8e48dfb` and `value_flow_v2#3ebf98b7`.
- `shard_state#9023afe2` and `split_state#5f327da5`.
- `_ config_addr:bits256 config:^(Hashmap 32 ^Cell) = ConfigParams`.
- Exotic `MERKLE_PROOF` tag `0x03` and `MERKLE_UPDATE` tag `0x04`.

The current Rust types parse stable constructor boundaries and preserve deeper
child cells by reference where full generated model expansion is still pending.
This keeps BoC bytes, root hashes, references, and exact reserialization stable
for LiteClient workflows.

## Invariants And Edge Cases

- `ShardIdent.shard_pfx_bits` must be `0..=60`.
- `Block` requires constructor tag `0x11ef55aa` and four referenced children.
- `ValueFlow` accepts only `0xb8e48dfb` or `0x3ebf98b7`.
- `ShardState` accepts unsplit `0x9023afe2` or split `0x5f327da5`.
- Merkle proof/update wrappers require exotic cells with one or two references.
- Proof helper verification only checks stored virtual hashes against child
  hashes. It is not a full liteserver trust check.
- Exact top-level TL-B decode rejects trailing data through `TlbDeserialize`.

## Current Crate Mapping

- `src/tlb/schema.rs` parses the Phase 1 schema slice and verifies the checked
  generated summary.
- `src/tlb/block.rs` implements `ShardIdent`, `ExtBlkRef`, `BlockIdExtTlb`,
  `Block`, `BlockExtra`, `ValueFlow`, `ShardState`, `ConfigParams`,
  `MerkleProof`, and `MerkleUpdate`.
- `src/liteclient/boc.rs` preserves raw LiteClient BoC bytes alongside decoded
  cells and typed views.
- `src/cli/mod.rs` exposes offline BoC/TL-B inspection and schema checks.

## Missing Work

Full generated expansion of `BlockInfo`, `ValueFlow`, `BlockExtra`,
`ShardStateUnsplit`, `McStateExtra`, config params, and masterchain extra
families remains follow-up work. Live or upstream-captured BoCs for block,
config, account-state, and proof paths are still backlog evidence; required
Phase 1 tests remain offline and synthetic.
