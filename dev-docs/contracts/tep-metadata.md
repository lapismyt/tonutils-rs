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

`src/metadata.rs` implements this common layer as `parse_tep64_content`:

- top-level `0x01` is decoded as off-chain URI content using snake bytes;
- top-level `0x00` is decoded as on-chain `HashmapE 256 ^Cell` content keyed
  by SHA-256 field-name hashes;
- on-chain values with in-value `0x00` decode as snake bytes;
- on-chain values with in-value `0x01` decode as chunked `HashmapE 32 ^Cell`
  content and concatenate chunks in ascending chunk-index order;
- unsupported top-level tags and unknown dictionary keys preserve the raw cell;
- malformed on-chain field values are preserved as field diagnostics instead of
  dropping the entire dictionary.

The recognized key set currently covers common TEP-64 fields used by jettons and
NFTs: `uri`, `name`, `description`, `image`, `image_data`, `symbol`,
`decimals`, `amount_style`, `render_type`, `content_url`, and `video`.

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

The common parser does not yet fetch off-chain JSON, merge semi-chain content,
or decode jetton/NFT get-method stacks. Those wrapper integrations remain
tracked in `TODO.md`.

## Sources

- Official TON token metadata documentation for TEP-64 content markers, snake
  encoding, chunked encoding, and common jetton/NFT metadata keys:
  <https://docs.ton.org/standard/tokens/metadata>.
