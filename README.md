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
- [Wallets](docs/wallets.md): derive addresses and build signed messages offline.
- [Documentation book](https://lapismyt.github.io/tonutils-rs/): browse all public
  guides plus the internal protocol notes.

## Crates

- `tonutils-crc`, `tonutils-tl`, `tonutils-tvm`, and `tonutils-tlb` provide the protocol foundation.
- `tonutils-macros` provides derive and procedural macros.
- `tonutils-schema-gen` generates code from TL schemas.
- `tonutils-adnl` and `tonutils-liteclient` provide ADNL TCP and LiteAPI access;
  `tonutils-adnl/udp` adds authenticated direct/channel UDP sessions.
- `tonutils-overlay` provides bounded overlay peer, routing, discovery-record,
  lifecycle, and status primitives.
- `tonutils-mempool` provides a low-latency Rust `Stream` for pending external messages.
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

## Mempool scanner

The mempool crates are a pending-message scanner, not a replacement for the
LiteServer block or indexing APIs. `tonutils-mempool` intentionally has no
WebSocket layer: applications consume a Rust `Stream`, while transport and
presentation remain separate. Its fast path validates the BoC envelope, hashes
and deduplicates shared raw bytes, then emits `ExternalMessage`; full TL-B
decoding and `Included` correlation belong to a separately configured slow path.

The current release exposes bounded peer/event primitives, signed discovery
record validation, a transport-neutral session manager, native UDP seed-only
delivery, FEC reassembly, and `MempoolScannerBuilder` bootstrap resolution.
The behavioral reference for this design is
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool), not a source
of Rust or Python API compatibility requirements.

```toml
[dependencies]
tonutils-mempool = "2"
tonutils-overlay = "2"
```

```rust,no_run
use futures::StreamExt;
use tonutils_mempool::{MempoolConfig, MempoolEvent, MempoolScanner};

# tokio::runtime::Runtime::new().unwrap().block_on(async {
let scanner = MempoolScanner::new(MempoolConfig::default())?;
let mut events = Box::pin(scanner.events());
let _ = scanner.ingest(
    [0xb5, 0xee, 0x9c, 0x72, 0x00],
    tonutils_overlay::RoutingMetadata::new(
        tonutils_overlay::OverlayId::from_name(b"local"),
        tonutils_overlay::PeerId::from_bytes([0; 32]),
    ),
).await?;
if let Some(MempoolEvent::ExternalMessage { hash, raw_boc, .. }) = events.next().await {
    println!("seen {hash:?} ({} bytes)", raw_boc.len());
}
# Ok::<(), Box<dyn std::error::Error>>(()) });
```
