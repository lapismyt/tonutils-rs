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

`src/jetton.rs` implements typed metadata support for TEP-74-compatible jetton
masters. The wrapper decodes the official `get_jetton_data()` stack layout:

- `total_supply` as a non-negative integer;
- `mintable` as `-1` for true and `0` for false;
- `admin_address` as a standard internal address or `addr_none`;
- `jetton_content` as a TEP-64 content cell;
- `jetton_wallet_code` as the returned wallet code cell.

`JettonMasterData::metadata()` maps the parsed TEP-64 content into
`JettonMetadata` fields for `uri`, `name`, `description`, `image`,
`image_data`, `symbol`, `decimals`, and `amount_style`. Unknown keys,
unsupported jetton-adjacent keys, raw content, and field-level malformed value
diagnostics remain inspectable. Off-chain JSON fetching is not implemented in
this layer; off-chain content only fills the discovered URI.

Required fixture coverage:

- Off-chain URI content: covered by deterministic unit tests.
- On-chain dictionary content: covered by deterministic unit tests.
- Unknown-key preservation: covered by deterministic unit tests.
- Malformed field diagnostics and top-level malformed content rejection:
  covered by deterministic unit tests.

The `Contract` helper `jetton_master_data_latest()` runs `get_jetton_data` at
the provider's latest masterchain block and decodes successful stacks.
`jetton_metadata_latest()` returns the mapped metadata. Mock-provider tests
cover method-id routing, latest-block lookup, provider errors, and non-zero
exit code propagation.

## NFT Metadata

`src/nft.rs` implements typed metadata support for TEP-62-compatible NFT
collections and items. The wrapper decodes the official get-method stack
layouts:

- `get_collection_data()` returns `next_item_index`, `collection_content`, and
  `owner_address`;
- `get_nft_data()` returns `init?`, `index`, `collection_address`,
  `owner_address`, and `individual_content`;
- `get_nft_content(index, individual_content)` returns a full TEP-64 content
  cell for a collection-backed item.

`NftCollectionData::metadata()` maps collection content into `NftMetadata`.
Standalone item metadata is supported by `nft_item_metadata_latest()` when
`collection_address` is `addr_none`; collection-backed item metadata should be
resolved by running `nft_full_item_metadata_latest(&item_data)` on the
collection contract so merge behavior remains contract-defined.

Address stack entries accept `addr_none` as `None` and standard internal
addresses as `Some(Address)`. External addresses, variable-length internal
addresses, anycast, trailing data, and malformed cells are structured decode
errors.

`NftMetadata` maps `uri`, `name`, `description`, `image`, `image_data`,
`render_type`, `content_url`, and `video`. Unknown keys and known but
NFT-unmapped jetton fields such as `symbol`, `decimals`, and `amount_style`
remain raw-preserved. Malformed known field values become field diagnostics
instead of invalidating the whole metadata object.

Required fixture coverage:

- Collection metadata: covered by deterministic unit tests.
- Item metadata: covered by deterministic unit tests.
- Individual-content merge behavior through `get_nft_content`: covered by
  deterministic mock-provider tests.
- Unknown-key preservation: covered by deterministic unit tests.
- Malformed content rejection and malformed field diagnostics: covered by
  deterministic unit tests.

## Current Limits

The common parser, jetton metadata mapper, and NFT metadata mapper do not fetch
off-chain JSON or merge semi-chain content locally. NFT transfers, royalty
helpers, `get_nft_address_by_index`, SBT extensions, and indexer API
integration remain out of scope for the current wrapper layer.

## Sources

- Official TON token metadata documentation for TEP-64 content markers, snake
  encoding, chunked encoding, and common jetton/NFT metadata keys:
  <https://docs.ton.org/standard/tokens/metadata>.
- Official TON jetton interface documentation for `get_jetton_data()` stack
  fields and `mintable` `-1/0` semantics:
  <https://docs.ton.org/standard/tokens/jettons/api>.
- Official TON NFT interface documentation for `get_collection_data()`,
  `get_nft_data()`, and `get_nft_content()` stack fields:
  <https://docs.ton.org/standard/tokens/nft/api>.
- Official TON NFT reference documentation for contract-defined full item
  content composition:
  <https://docs.ton.org/standard/tokens/nft/nft-reference>.
