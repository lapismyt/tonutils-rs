# tonutils-rs

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils-tvm.svg)](https://crates.io/crates/tonutils-tvm)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)

`tonutils-rs` is a pure-Rust TON SDK workspace. Version 2.0 removes the monolithic
`tonutils` facade: depend only on the focused `tonutils-*` crates your project
uses.

## Crates

- `tonutils-crc`, `tonutils-tl`, `tonutils-tvm`, and `tonutils-tlb` provide the protocol foundation.
- `tonutils-macros` provides derive and procedural macros.
- `tonutils-schema-gen` generates code from TL schemas.
- `tonutils-adnl` and `tonutils-liteclient` provide ADNL TCP and LiteAPI access.
- `tonutils-network-config` parses global configuration independently.
- `tonutils-contracts` contains raw provider traits, get-method helpers, and state-init blueprints.
- `tonutils-metadata`, `tonutils-jetton`, `tonutils-nft`, and `tonutils-wallet` provide application-level offline helpers.
- `tonutils-cli` supplies the `tonutils` command-line executable.

## Install

```toml
[dependencies]
tonutils-tvm = "2.0.0"
tonutils-tlb = "2.0.0"
```

```rust
use tonutils_tvm::{Builder, serialize_boc};
```

For LiteAPI applications, add `tonutils-liteclient` and, when global config
parsing is needed, enable its default `network-config` feature or add
`tonutils-network-config` directly. Wallet, jetton, and NFT payload APIs work
offline by default; their provider extensions use the respective `provider`
feature.

## Development

```bash
cargo fmt --check
cargo check --workspace --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Protocol fixtures and internal design notes live in `dev-docs/`. Proof payload
preservation is not equivalent to trust-level proof verification.
