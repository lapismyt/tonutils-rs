# tonutils-rs

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils.svg)](https://crates.io/crates/tonutils)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)

`tonutils-rs` is a pure-Rust TON SDK workspace. The `tonutils` runtime facade
groups the focused `tonutils-*` crates behind one dependency, while each focused
crate remains available for applications that prefer narrower dependencies.

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

Add the facade with Cargo:

```bash
cargo add tonutils
```

Or add it directly to `Cargo.toml`:

```toml
[dependencies]
tonutils = "2"
```

```rust
use tonutils::{Builder, serialize_boc};
```

The facade exports runtime crates as `tonutils::tvm`, `tonutils::tlb`,
`tonutils::liteclient`, and so on. Its default features enable ADNL TCP and
LiteAPI network-config support; provider integrations can be enabled with
`jetton-provider`, `nft-provider`, and `wallet-provider`.

Protocol fixtures and internal design notes live in `dev-docs/`. Proof payload
preservation is not equivalent to trust-level proof verification.
