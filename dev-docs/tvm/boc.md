# Bag Of Cells

## Purpose And Scope

Bag of Cells serializes a directed acyclic graph of TON cells into bytes. This
page documents the ordinary-cell BoC baseline implemented in `src/tvm/boc.rs`.
The current implementation is intended to support future TL-B and contract work
without depending on a third-party Rust TON SDK.

This slice covers generic BoC decoding and encoding for ordinary and supported
exotic cells, optional CRC32, optional index tables during decode, and string
conversion helpers.

## Wire Format

The supported generic BoC magic is:

```text
b5 ee 9c 72
```

The generic header fields are read in this order:

```text
magic:4
flags_and_size:1
offset_bytes:1
cells_count:size_bytes
roots_count:size_bytes
absent_count:size_bytes
cells_size:offset_bytes
root_index:size_bytes
index_table:cells_count * offset_bytes, only when has_idx is set
cells:cells_size
crc32:4, only when has_crc32 is set
```

`flags_and_size` contains:

- bit 7: index table flag,
- bit 6: CRC32 flag,
- bit 5: cache bits flag,
- low 3 bits: `size_bytes`.

Integer fields are big-endian. The CRC32 trailer used by this crate is stored
little-endian, matching the existing serializer behavior.

## Cell Serialization

Each cell serializes as:

```text
refs_descriptor:1
bits_descriptor:1
data:ceil(bits / 8)
ref_indexes:ref_count * size_bytes
```

The first descriptor stores reference count in bits `0..=2`, the exotic flag in
bit `3`, and level in bits `5..=6`. Reserved bits must be unset. For ordinary
cells, the level is the maximum level of all references. For exotic cells, the
decoder derives the level from the parsed exotic kind and rejects a descriptor
whose level does not match.

The second descriptor is `floor(bits / 8) + ceil(bits / 8)`. For partial-byte
cell data, the serialized data includes a top-up `1` bit immediately after the
last data bit and zero padding after that marker. The decoder removes the
top-up marker and restores the exact bit length.

Reference indexes are read using `size_bytes`, not a hard-coded one-byte index.
Indexes must point to a parsed cell index.

Supported exotic payloads:

- pruned branch: tag `0x01`, mask `1..=7`, one hash and one depth per set mask
  bit, no references,
- library reference: tag `0x02`, one 32-byte hash, no references,
- Merkle proof: tag `0x03`, one proof hash and proof depth, one reference,
- Merkle update: tag `0x04`, old/new proof hashes and depths, two references.

The decoder preserves exotic cells as `Cell::exotic_kind()` instead of
rebuilding them as ordinary cells. Unsupported tags and invalid kind-specific
payloads fail decoding with an `Invalid exotic cell` error context.

## Invariants And Edge Cases

The strict semantic decoders currently accept:

- one or more roots through `deserialize_boc_roots()`, with `deserialize_boc()`
  reserved for exactly one root and rejecting multi-root payloads,
- zero absent cells,
- generic BoC magic `b5ee9c72`,
- ordinary cells with up to four references,
- optional index tables when `has_idx` is set,
- optional CRC32 trailers,
- supported exotic cells with exact payload lengths and reference counts,
- hex and standard base64 string wrappers through `hex_to_boc()` and
  `base64_to_boc()`.

Both strict semantic decoders reject:

- unknown magic values,
- legacy indexed magic values that are not the generic BoC layout,
- truncated header or cell data,
- `size_bytes` or `offset_bytes` outside `1..=8`,
- cache-bit BoCs with the explicit error that cache bits are unsupported for
  ordinary-cell decoding,
- cell descriptors with reserved bits set,
- exotic-cell descriptor levels that do not match the level derived from the
  exotic payload and references,
- unsupported exotic-cell type tags,
- invalid exotic-cell payloads, including short type tags, invalid pruned masks,
  wrong payload lengths, and wrong reference counts,
- malformed index tables, including non-monotonic offsets or a final offset
  that does not equal `cells_size`,
- root index out of range,
- reference index out of range,
- malformed partial-byte top-up markers,
- CRC32 mismatch,
- trailing bytes after the cell payload when CRC32 is absent, or after the CRC32
  trailer when it is present.

`deserialize_boc()` additionally rejects otherwise valid BoCs that contain zero
roots or more than one root. Use `deserialize_boc_roots()` when a semantic
caller expects a strict multi-root cell set.

`inspect_boc()` is a proof-oriented structural path. It parses the same generic
header, root indexes, optional index table, CRC32 trailer, cell descriptors,
raw serialized cell payloads, and reference indexes, then computes root
representation hashes from the descriptor bytes, raw serialized data, child
depths, and child hashes. It does not construct `Cell` values and therefore
does not validate exotic-cell tags, exotic payload lengths, TL-B types, proof
paths, or trust roots. Structural failures such as invalid root indexes,
invalid reference indexes, CRC32 mismatches, truncated payloads, cache-bit
payloads, and reserved descriptor bits still fail.

## Crate Mapping

- `serialize_boc(root, has_crc32)` writes generic BoCs without an index table.
- `deserialize_boc(data)` reads strict single-root generic BoCs with or without
  index tables and preserves supported exotic-cell kinds.
- `deserialize_boc_roots(data)` returns all strict semantic root cells for
  multi-root payloads.
- `inspect_boc(data)` returns proof diagnostic root counts and hashes without
  requiring semantic proof verification.
- `hex_to_boc()` and `boc_to_hex()` wrap byte-level BoC conversion.
- `base64_to_boc()` and `boc_to_base64()` use standard base64 for BoC strings.

## Fixture Coverage

Current embedded fixture tests cover:

- empty ordinary generic BoC,
- one-byte ordinary generic BoC,
- ordinary generic BoC with one reference, with and without an index table,
- exotic library-reference BoC,
- multi-root proof diagnostic inspection,
- malformed cache-bit BoC rejection.

These fixtures are intentionally small hex constants in `src/tvm/boc.rs`. They
are derived from the TON `serialized_boc#b5ee9c72` layout and cross-check the
crate's canonical serializer output for supported cases.

Phase 1 TL-B compatibility fixtures in `fixtures/phase1/` add checked-in BoC
hex payloads with metadata for message, account, transaction, and
transaction-description roots. Their tests decode the BoC, compare the root
representation hash from the cell, decode the declared TL-B type, and require
canonical serializer output to match the fixture bytes exactly. They are
offline-only and do not use live network access.

## Cache-Bit Policy

Cache-bit BoCs remain unsupported in this crate. The generic BoC header can
signal cache bits, but the current SDK has no public representation for cached
hash/depth material and no fixture proving that ignoring those bits is lossless
for all supported cell kinds. Decoding such payloads by silently discarding the
extra metadata would make compatibility ambiguous, especially for future proof
and archive workflows.

Until a concrete upstream fixture requires cache-bit preservation, the decoder
rejects these BoCs with `BoC cache bits flag is unsupported for ordinary-cell
decoding`. Serialization always writes the cache-bit flag as zero.

## Missing Work

- Multi-level exotic hash/depth helper APIs for proof verification.
- Legacy indexed magic variants `68ff65f3` and `acc3a728`.
- Additional captured golden BoCs from upstream TON, pytoniq-core, or public
  liteservers for deep account-proof, block-proof, and config proof cells.
- Encoding with an index table when callers need that output mode.
- Full proof verification for account and block proof BoCs.
