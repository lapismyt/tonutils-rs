# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
