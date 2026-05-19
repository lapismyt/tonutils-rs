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

TL-B offline fixtures are embedded in `src/tlb/mod.rs` under
`offline_fixture_tests`. They use a small metadata harness with:

- fixture name,
- source note,
- hex or base64 BoC payload,
- expected root representation hash,
- expected decoded TL-B type.

The current TL-B fixture set is synthetic and schema-derived from the
hand-written models already implemented in this crate. It covers:

- `Message Any` and `MessageRelaxed Any` with referenced children and exact
  trailing-data rejection,
- `StateInit`,
- `CurrencyCollection` with an extra-currency `HashmapE 32`,
- `Transaction` and `TransactionDescr`,
- `Account`,
- `ShardAccounts`, `AccountBlock`, and `ShardAccountBlocks`,
- standalone `HashmapE` canonical root-reference and label behavior,
- standalone `HashmapAugE` empty top-level extras, non-empty top-level extras,
  leaf extras, and fork extras.

These fixtures are intentionally offline-only. They lock canonical
decode/encode/hash behavior for the current model surface without claiming that
the values were captured from a live liteserver or copied from upstream test
data.

Phase 1 milestone fixtures are also checked in under `fixtures/phase1/` as JSON
metadata plus small BoC hex payloads. Normal `cargo test --lib` runs remain
fully offline: tests read these files with `include_str!`, decode the BoCs,
check source metadata, compare root representation hashes, decode the expected
TL-B type, and require canonical reserialization back to the exact fixture
bytes.

Current checked-in Phase 1 files:

- `fixtures/phase1/account_message_transaction.json`: `Message Any`,
  `MessageRelaxed Any`, `Transaction`, and `Account` fixtures.
- `fixtures/phase1/transaction_descriptions.json`: transaction-description
  fixtures for ordinary, tick-tock, split prepare, split install, merge prepare,
  and merge install constructors.

These fixtures are synthetic but upstream-schema-derived. They are generated
from the local hand-written codecs that map directly to the documented upstream
TL-B layouts. Live/public liteserver captures and upstream repository capture
vectors remain useful as stronger evidence for later compatibility expansion,
but they are not required by normal test runs and must never make CI depend on
network access.

ABI golden fixtures are checked in under `fixtures/abi/contracts.json` as JSON
metadata and small BoC hex payloads. The schema records:

- `schema_revision`,
- top-level synthetic `source` and `capture_date`,
- fixture `name`, `kind`, `source`, ABI JSON document, and function name,
- evidence metadata: `evidence_kind`, `source_url`, `source_commit`,
  `network`, `account`, `block_id`, `method_id`, `capture_command`, and
  `compat_reference`,
- fixture-only ABI input values,
- expected stack BoC/root hashes for get-method inputs and outputs, or expected
  message-body BoC/root hash for body codecs,
- expected decoded ABI values.

Normal ABI fixture tests read the file with `include_str!`, load each ABI JSON
document through `parse_abi_json_str`, encode and decode through the existing
ABI stack/message-body helpers, and verify exact BoC hex plus root
representation hashes. The message-body entries are body-cell BoCs only, not
full message envelopes.

The current ABI fixture set is synthetic offline evidence plus
`captured_or_opt_in` templates. It covers one get-method stack vector with
address input and uint output, one opcode-prefixed external body vector with
scalar and referenced values, one no-selector internal body vector with tuple
and optional values, and local map/dictionary roundtrips for fixed integer-key
maps. The opt-in templates cover wallet `seqno` with no inputs and TEP-74
`get_wallet_address(owner)` with one address input; they are not claimed as
captured evidence until live stack/result bytes and block/account metadata are
filled in.

TVM stack golden fixtures are checked in under `fixtures/tvm/stack.json`. The
schema records:

- `schema_revision`,
- synthetic `source` and `capture_date`,
- fixture `name`,
- `input_stack_boc_hex`,
- root representation hash,
- decoded stack entry shape,
- `captured_or_opt_in` live-capture template metadata,
- `cross_sdk_vectors` for tonutils-go raw params BoC matches or tonlib
  structural comparisons when those bytes are available.

The stack fixture set is synthetic offline evidence generated by the local
stack codec. It covers non-empty scalar stacks, linked stack chains beyond four
logical entries, nested tuple/list values, huge integers, cell/slice entries,
and unsupported raw bytes. Normal fixture tests decode each BoC, compare the
expected entries and root hash, and verify exact canonical reserialization.
Ignored live smoke tests can add transport evidence without making CI depend on
network access. The ignored non-empty stack test prints fixture JSON only after
`exit_code == 0`; non-zero exit-code smoke checks do not produce captured
fixture material.

## Pending Captured Fixtures

Live or upstream-captured BoCs remain required before claiming broader
wire-level compatibility for deep account-proof, block-proof, and config
workflows. When added, captured fixtures should record:

- liteserver endpoint or upstream repository/commit,
- capture date,
- relevant schema revision,
- source command or script,
- expected decoded type,
- expected root hash and, when available, file hash,
- whether the fixture is required in normal test runs or kept as ignored
  captured evidence.
