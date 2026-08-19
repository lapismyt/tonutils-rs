# tonutils-rs: a pure-Rust TON SDK for Rust applications

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils.svg)](https://crates.io/crates/tonutils)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)

`tonutils-rs` is a pure-Rust SDK for building TON applications. It covers
offline TVM and TL-B data, TL/LiteAPI serialization, native ADNL and LiteClient
network access, contract helpers, wallets, jettons, NFTs, and a scriptable CLI.

Use the `tonutils` facade when an application spans several runtime layers, or
depend on a focused `tonutils-*` crate when compile time and feature control
matter. Protocol and serialization workflows are deterministic and offline;
network reads and submissions require an explicitly configured live provider.

## Start Here

- [Getting started](docs/getting-started.md): choose crates and Cargo features.
- [Examples](docs/examples.md): compile offline workflows and run live queries.
- [TVM primitives](docs/tvm.md): cells, BoC, addresses, dictionaries, and stacks.
- [LiteClient](docs/liteclient.md): connect to TON liteservers over LiteAPI.
- [Wallets](docs/wallets.md): derive addresses and build signed messages offline.
- [Documentation book](https://lapismyt.github.io/tonutils-rs/): browse all public
  guides plus the internal protocol notes.

## Crates

- `tonutils-crc`, `tonutils-tl`, `tonutils-tvm`, and `tonutils-tlb` provide the protocol foundation.
- `tonutils-macros` provides derive and procedural macros.
- `tonutils-schema-gen` generates code from TL schemas.
- `tonutils-adnl` and `tonutils-liteclient` provide ADNL TCP and LiteAPI access.
- `tonutils-network-config` parses global configuration independently.
- `tonutils-contracts` contains raw provider traits, get-method helpers, and state-init blueprints.
- `tonutils-metadata`, `tonutils-jetton`, `tonutils-nft`, and `tonutils-wallet` provide application-level offline helpers.
- `tonutils-cli` supplies the `tonutils` command-line executable.

## Install the facade

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
`tonutils::liteclient`, and so on. Its default features enable native ADNL TCP
and LiteAPI network-config support; provider integrations can be enabled with
`jetton-provider`, `nft-provider`, and `wallet-provider`.

Protocol fixtures and internal design notes live in `docs/reference/`. Proof payload
preservation is not equivalent to trust-level proof verification.
