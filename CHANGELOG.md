# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Planned as `2.0.0`. This section describes the net changes since `1.1.0` and is
not a release tag or release date.

### Added

- Added an encrypted ADNL UDP transport with sessions, channel ciphers,
  query/receive paths, and capture helpers, alongside the existing TCP peer.
- Added ADNL wire types under `adnl::tl` and typed LiteAPI request/response
  types under `liteclient::tl`.
- Added `tonutils-schema-gen`, a deterministic TL/TL-B inventory tool with a
  pinned upstream schema snapshot, constructor inventory, SHA-256 drift checks,
  and checked-in generated metadata.
- Added typed TL-B coverage for blocks, shard state, accounts, transactions,
  messages, and proof structures, including compatibility fixtures.
- Added live liteserver CI coverage and populated get-method result stacks.
- Added the `provider` feature with capability traits for masterchain info,
  account state, get-methods, external messages, and transaction history.
- Added TonAPI, Toncenter v2, Toncenter v3, LiteClient, and LiteBalancer
  provider adapters with configurable endpoints and credentials.
- Added CLI provider selection (`--provider`), self-hosted endpoint, API key,
  and timeout flags for high-level commands.
- Added the opt-in `emulator` feature with runtime-loaded `libemulator`
  wrappers for transaction, tick/tock, get-method, and message emulation,
  including typed results, configurable builders, and raw JSON escape hatches.
- Added the `emulator-download` feature, which downloads a released upstream
  `libemulator` shared library during `cargo build` and embeds its path for
  `EmulatorEngine::open_default`.
- Added ignored live emulator tests against a wallet v4r2 code/data fixture,
  plus a CI job that runs them with the downloaded library.
- Added the upstream-compatible `tvm::VmStack`, `VmStackValue`, and `VmSlice`
  standard stack codec, with conversions from the existing `TvmStack`.

### Changed

- Consolidated the SDK into one feature-gated `tonutils` crate with opt-in
  modules (`tvm`, `tl`, `tlb`, `wallet`, `crc`, `metadata`, `network-config`,
  `adnl`, `liteclient`, `provider`, `contracts`, `serde`); the default feature
  set is now empty. `tonutils-cli`, `tonutils-macros`, and `tonutils-schema-gen` remain
  separate packages.
- Replaced the previous feature model (`std`, `adnl-tcp`, `tlb-derive`,
  `contract-derive`, `abi-json`, `cli`, `full`) with the per-module features.
- Bumped the workspace to `2.0.0`, edition 2024, and MSRV 1.85, with major
  dependency upgrades (`ed25519-dalek` 3, `curve25519-dalek` 5, `syn` 3,
  `num-bigint` 0.5, `base64` 0.23).
- Refresh crate documentation, README, and CLI layout for the new feature and
  crate boundaries.
- Switched Bag-of-Cells checksum generation and validation from CRC32/ISO-HDLC
  to the protocol-correct CRC32C required by TON and `libemulator`.
- Switched get-method identifier hashing from CRC16/IBM-SDLC to the
  TON-compatible CRC16/XMODEM.

### Fixed

- Fixed Bag-of-Cells serialization to emit parent cells before their
  references; upstream TVM rejects the previous child-first ordering.

### Removed

- Removed the ABI API, documentation, fixtures, and ABI-specific CLI commands.
- Removed the jetton and NFT modules.
- Removed the `contract-derive`/`Contract` derive and the legacy `utils`
  helpers.
- Removed the unreleased overlay/mempool scanner, DHT, and QUIC transport work.

### Migration

- Enable every module your application uses explicitly; no features are enabled
  by default.
- Replace `adnl-tcp` with `adnl`, `tlb-derive` with `tlb`, and remove `abi-json`
  and `cli` from library features. Use the separate `tonutils-cli` package for
  the binary.
- Applications relying on ABI, jetton, or NFT modules must retain a local
  implementation or wait for a separately scoped release.

## [1.1.0] - 2026-06-02

### Added

- Added public ABI payload component helpers for selector-free payload encoding
  and exact decoding.
- Added ABI event payload encode/decode helpers using the same local wire policy
  as message bodies.
- Added synthetic event payload fixture coverage and opt-in ABI workflow
  acceptance coverage for wallet `seqno` and TEP-74 `get_wallet_address`.

### Changed

- Allowed the release workflow to publish the current version.
- Improved offline test coverage, README badges, and published documentation.

## [1.0.0] - 2026-05-19

### Added

- First stable release: TVM cells, slices, builders, BoC, addresses,
  dictionaries, and stacks; TL/TL-B types and derive support; ADNL primitives
  and TCP transport; LiteClient and LiteBalancer; network configuration;
  contracts and provider boundaries; Wallet V4R2/V5R1; metadata parsing; jetton
  and NFT payload builders; ABI; and the `tonutils` CLI.
- Added GitHub Actions automation for ignored live-network tests and GitHub
  Release publishing for crates.io releases.

### Changed

- Set the `tonutils` and `tonutils-tlb-derive` crate versions to `1.0.0`.
- Made `tonutils-tlb-derive` publishable and versioned the root crate's optional
  path dependency for crates.io packaging.

[Unreleased]: https://github.com/lapismyt/tonutils-rs/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/lapismyt/tonutils-rs/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/lapismyt/tonutils-rs/releases/tag/v1.0.0
