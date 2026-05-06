# Fixture Policy

Fixtures are required for compatibility with official TON behavior.

## Fixture Metadata

Every fixture should document:

- source,
- date,
- upstream commit if known,
- schema file version,
- expected decoded structure,
- whether it is synthetic or captured.

## Fixture Types

- TL binary constructors.
- ADNL frames.
- BoC files.
- Cell hashes.
- Account states.
- Block proofs.
- Get-method results.

## Storage Rules

- Keep binary fixtures small.
- Prefer hex for very small values.
- Use files for larger BoC or network captures.
- Never include private keys or sensitive live credentials.

## Current Embedded Fixtures

Address fixtures are embedded in `src/tvm/address.rs` because they are short
text vectors:

- TON Docs internal address formats page, accessed 2026-05-06:
  raw address
  `0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e`
  with bounceable, non-bounceable, bounceable test-only, and non-bounceable
  test-only user-friendly forms. These vectors validate tag handling,
  URL-safe and standard base64 alphabets, CRC16 validation, and raw conversion.
- Zero-address fixture
  `0:0000000000000000000000000000000000000000000000000000000000000000`
  with known bounceable and non-bounceable user-friendly encodings. This vector
  is a synthetic edge case generated from the TEP-0002 36-byte address layout
  documented by TON Docs.

BoC fixtures are embedded in `src/tvm/boc.rs` as small hex constants:

- empty ordinary cell: validates the generic `b5ee9c72` header and zero-bit
  cell descriptors,
- one-byte ordinary cell: validates byte-aligned cell payload descriptors,
- one-reference ordinary cell with and without an index table: validates root
  indexes, reference indexes, and canonical reserialization without index
  output,
- library-reference exotic cell: validates supported exotic tag `0x02`,
  descriptor preservation, and exact payload length,
- cache-bit variant of the empty-cell BoC: validates the crate policy that
  cache-bit BoCs are rejected until a lossless semantic need is proven.

The BoC byte vectors are synthetic but schema-derived from the TON Blockchain
paper serialized BoC constructor `serialized_boc#b5ee9c72`; no third-party Rust
TON SDK code or fixture dependency is used.
