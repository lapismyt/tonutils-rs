# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

This section describes the breaking changes being prepared for `2.0.0`. It is
not a release tag or release date.

### Added

- Added independent `tonutils-*` crates for ADNL, contracts, CRC, jettons,
  LiteClient, metadata, network configuration, NFTs, TL, TL-B, TVM, wallets,
  macros, and deterministic schema generation.
- Added provider-focused contract and wrapper boundaries, including dedicated
  provider APIs for contract, wallet, jetton, NFT, and LiteClient workflows.
- Added `tonutils-schema-gen`, a deterministic TL/TL-B source inventory tool
  with SHA-256 drift checks and checked-in generated constructor metadata.
- Added generated metadata for the tracked `lite_api.tl`, `ton_api.tl`,
  `tonlib_api.tl`, and block TL-B schema snapshots.

### Changed

- Replaced the monolithic `tonutils` library with a modular workspace of
  independently consumable `tonutils-*` crates, all currently prepared at
  workspace version `2.0.0`.
- Updated examples, documentation, package manifests, release automation, and
  CI to use the modular crate boundaries and workspace feature checks.
- Updated workspace dependency versions and the lockfile after `v1.1.0`.

### Removed

- Removed the ABI API, ABI documentation, ABI fixtures, and ABI-specific CLI
  commands from the workspace.
- Removed the monolithic crate entry points and compatibility layout that
  previously exposed those APIs.

### Migration

- Replace the `tonutils` dependency with the specialized `tonutils-*` crates
  needed by your application; use their public crate roots rather than the
  former monolithic module paths.
- Applications using the removed ABI API or CLI commands must retain a local
  implementation or wait for a future, separately scoped ABI initiative.

## [1.1.0] - 2026-06-02

### Added

- Added public ABI payload component helpers for selector-free payload encode
  and exact decode.
- Added ABI event payload encode/decode helpers using the same local wire policy
  as message bodies.
- Added synthetic event payload fixture coverage and opt-in ABI workflow
  acceptance coverage for wallet `seqno` and TEP-74 `get_wallet_address`.

## [1.0.0] - 2026-05-19

### Added

- Added GitHub Actions automation for ignored live-network tests.
- Added GitHub Release publishing automation for crates.io releases.

### Changed

- Set the `tonutils` and `tonutils-tlb-derive` crate versions to `1.0.0`.
- Made `tonutils-tlb-derive` publishable and versioned the root crate's optional
  path dependency for crates.io packaging.
