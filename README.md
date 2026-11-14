# tonutils

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils.svg)](https://crates.io/crates/tonutils)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)

> [!NOTE]
> Unfortunately I can't dedicate much time to this project, so most of the code is AI-generated.

`tonutils` is a pure-Rust SDK for The Open Network (TON). It implements TON
cells, Bag of Cells (BoC), TL-B structures, TL/LiteAPI wire types, ADNL,
wallets, and a direct liteserver client without depending on
a third-party TON SDK or a runtime native library.

Features are opt-in and the default feature set is empty. Enable only the
layers your application uses.

## Quickstart

For offline cells, TL-B structures, and wallet construction:

```toml
[dependencies]
tonutils = { version = "2.1", features = ["tvm", "tlb", "wallet"] }
```

For a direct liteserver client, add the network layers you need:

```toml
[dependencies]
tonutils = { version = "2.1", features = ["liteclient"] }
```

For provider-neutral reads across liteservers, TonAPI, and Toncenter:

```toml
[dependencies]
tonutils = { version = "2.1", features = ["provider"] }
```

```rust
use tonutils::provider::{GetMasterchainInfoExt, TonApi};

async fn latest_seqno() -> Result<i32, Box<dyn std::error::Error>> {
    let mut provider = TonApi::mainnet();
    let info = provider.get_masterchain_info().await?;
    Ok(info.last.seqno)
}
```

### Offline Cell Example

```rust
use tonutils::tvm::{Builder, deserialize_boc, serialize_boc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::new();
    builder.store_u32(0xdead_beef)?;

    let root = builder.build()?;
    let boc = serialize_boc(&root, true)?;
    let decoded = deserialize_boc(&boc)?;

    assert_eq!(root.hash(), decoded.hash());
    println!("encoded {} bytes", boc.len());
    Ok(())
}
```

### Live Read Shape

```rust
use tonutils::{liteclient::LiteClient, network_config::ConfigGlobal};

async fn latest_masterchain_seqno(
    config_json: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let config: ConfigGlobal = config_json.parse()?;
    let mut client = LiteClient::connect_first(&config).await?;
    let info = client.get_masterchain_info().await?;
    Ok(info.last.seqno)
}
```

The live example requires a Tokio runtime and application-selected configuration.
It is intentionally not a proof that a liteserver answered honestly.

## Features

| Feature | Provides | Requires live network |
| --- | --- | --- |
| `tvm` | Cells, builders, slices, BoC, addresses, dictionaries, TVM stacks, and `TvmError` | No |
| `tl` | Shared TL primitives, TL schema snapshots, generated metadata, and schema helpers | No |
| `tlb` | Typed TL-B blocks, messages, transactions, accounts, and proof structures | No |
| `wallet` | TON mnemonic handling, Ed25519 signing, Wallet V4R2/V5R1 addresses, and external message BoCs | No |
| `crc` | CRC16, CRC32, and CRC32C helpers used by TON identifiers and BoC formats | No |
| `emulator` | Runtime-loaded `libemulator` wrappers for transactions, tick/tock, get-methods, and message execution | No, but requires `libemulator` at runtime |
| `emulator-download` | `emulator` plus a build-time download of the latest upstream `libemulator` shared library for the target platform | Only during `cargo build` |
| `metadata` | TEP-64 on-chain and off-chain content parsing and encoding | No |
| `jetton` | TEP-74 jetton master and wallet wrappers, message bodies, get-methods, and embedded default code | Only for get-methods |
| `nft` | TEP-62/TEP-85 collection and item wrappers, message bodies, get-methods, and embedded default code | Only for get-methods |
| `network-config` | Global network configuration parsing and liteserver selection | No |
| `adnl` | ADNL identity, encrypted UDP transport, QUIC transport, and ADNL TL wire types | Yes for transport |
| `liteclient` | LiteAPI types, direct client, balancer, and typed BoC decode helpers | Yes |
| `provider` | Capability traits for liteservers, TonAPI, Toncenter v2/v3, and their HTTP/lite adapters | Yes |
| `contracts` | Contract wrapper instrumentation (`ContractDefinition`, typed data, offline `StateInit`/address, deploy requests), provider-neutral reads, and get-method stack helpers | Yes for reads |
| `serde` | Serde support for enabled public types that expose it, currently ADNL public keys | No |

The `emulator` feature is the only module that needs a native library at
runtime. It loads `libemulator` with `libloading` and never builds C++ during
`cargo build`; set `TONUTILS_EMULATOR_LIB` or pass an explicit path to
`EmulatorEngine::open`. Emulator inputs and outputs use the standard
`tvm::VmStack` codec. The optional `emulator-download` feature fetches a released
shared library during `cargo build` (pin with `TONUTILS_EMULATOR_RELEASE`) and
embeds its path for `EmulatorEngine::open_default`. Upstream publishes only
shared libraries, so this does not statically link `libemulator`. See
[docs/reference/emulator.md](docs/reference/emulator.md).

The `jetton` and `nft` features provide edition-specific wrappers because the
reference contracts differ in storage and get-method shapes. Every wrapper
supports embedded default code, custom code, offline address calculation,
deploy requests, and typed get-methods; message bodies are plain TL-B values.
Wallet V4R2 and V5R1 can attach a deploy `StateInit` to an outgoing message, so
one signed external message can deploy and fund a contract. See
[docs/reference/jetton-nft.md](docs/reference/jetton-nft.md) for layouts and
address rules.

The `provider` feature exposes separate `GetMasterchainInfoExt`, `GetAccountStateExt`,
`RunGetMethodExt`, `SendExternalMessageExt`, and `GetTransactionsExt` traits. HTTP
providers trust the remote service and do not verify Merkle proofs; liteserver
adapters also return responses without validating proof roots. The CLI selects a
provider with `--provider lite|tonapi|toncenter-v2|toncenter-v3`, and accepts
`--provider-endpoint`, `--provider-api-key`, and `--provider-timeout` for
self-hosted instances. Raw `liteclient`, `balancer`, and `contract` commands
require `--provider lite`.

`tl` owns the snapshots and shared primitives, not every TON protocol schema.
Handwritten ADNL and LiteAPI wire types live in `adnl::tl` and
`liteclient::tl` respectively. The ownership map and wire
invariants are documented in [docs/reference/README.md](docs/reference/README.md).

## Workspace

- `tonutils` is the library.
- `tonutils-cli` provides the `tonutils` command-line binary.
- `tonutils-macros` provides TL-B derive macros.
- `tonutils-schema-gen` validates checked-in schema metadata and inventory.

Use the crate's rustdoc for the public API. Repository-level development and
verification instructions are in [CONTRIBUTING.md](CONTRIBUTING.md). The crate
is licensed under MIT.
