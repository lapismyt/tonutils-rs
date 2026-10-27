# tonutils-rs: a pure-Rust TON SDK for Rust applications

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils.svg)](https://crates.io/crates/tonutils)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)

`tonutils-rs` is a pure-Rust SDK for building TON applications. It covers
offline TVM and TL-B data, TL/LiteAPI serialization, native ADNL and LiteClient
network access, overlay/mempool primitives, contract helpers, wallets, jettons,
NFTs, and a scriptable CLI.

Use the `tonutils` facade when an application spans several runtime layers, or
depend on a focused `tonutils-*` crate when compile time and feature control
matter. Protocol and serialization workflows are deterministic and offline;
network reads and submissions require an explicitly configured live provider.

## Start Here

- [Getting started](docs/getting-started.md): choose crates and Cargo features.
- [Examples](docs/examples.md): compile offline workflows and run live queries.
- [TVM primitives](docs/tvm.md): cells, BoC, addresses, dictionaries, and stacks.
- [LiteClient](docs/liteclient.md): connect to TON liteservers over LiteAPI.
- [Mempool](docs/mempool.md): stream pending external messages from the live overlay.
- [Wallets](docs/wallets.md): derive addresses and build signed messages offline.
- [Documentation book](https://lapismyt.github.io/tonutils-rs/): browse all public
  guides plus the internal protocol notes.

## Crates

- `tonutils` is the libary itself (public API)
- `tonutils-macros` provides derive and procedural macros.
- `tonutils-schema-gen` generates code from TL schemas.
- `tonutils-cli` supplies the `tonutils` command-line executable.

## Install

Add the library with Cargo:

```bash
cargo add tonutils
```

Or add it directly to `Cargo.toml`:

```toml
[dependencies]
tonutils = "2"
```
