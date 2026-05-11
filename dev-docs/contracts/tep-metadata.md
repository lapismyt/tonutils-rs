# TEP Metadata Roadmap

This page records the planned metadata parsing work for Phase 2 wrappers.
Implementation is tracked in `TODO.md`; this document keeps standards and
scope visible before parser code lands.

## Standards

- TEP-64 defines token and NFT metadata content layouts, including off-chain URI
  content and on-chain dictionary content.
- TEP-74 defines jetton contract interfaces whose wrappers need metadata from
  jetton master data.
- TEP-62 defines NFT item and collection interfaces whose wrappers need
  collection and item metadata.

The parser should use upstream TON and official TEP behavior as protocol truth.
SDKs such as pytoniq, pytoniq-core, tonutils-go, tongo, and STON.fi ton-rs are
comparison references only.

## Initial Parser Shape

The first metadata layer should be common and raw-preserving:

- Decode snake-cell byte strings.
- Decode chunked content dictionaries.
- Distinguish on-chain content, off-chain URI content, and unsupported content.
- Preserve raw cells and unknown keys so future TEP extensions do not become
  lossy decode failures.
- Surface malformed content as structured errors with enough context for users
  to inspect the original cell.

Jetton and NFT wrappers should build on that common layer instead of duplicating
metadata parsing.

## Jetton Metadata

Jetton support should decode typed get-method output for TEP-74-compatible
masters, then parse TEP-64 content into typed fields where the standard defines
them. Unknown keys and unsupported value encodings must remain available as raw
metadata entries.

Required fixture coverage:

- Off-chain URI content.
- On-chain dictionary content.
- Unknown-key preservation.
- Malformed snake and chunked content rejection.

## NFT Metadata

NFT support should decode typed get-method output for TEP-62-compatible
collections and items, then parse TEP-64 collection and item content. Item
metadata must leave room for individual-content merge behavior where collection
data and item data are combined by contract convention.

Required fixture coverage:

- Collection metadata.
- Item metadata.
- Individual-content merge behavior.
- Unknown-key preservation.
- Malformed content rejection.

## Current Limits

No metadata parser is implemented yet. The Phase 2 roadmap and `TODO.md` now
track the work so wallet implementation and ABI work do not hide jetton and NFT
metadata requirements.
