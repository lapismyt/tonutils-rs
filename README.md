# tonutils-rs

`tonutils-rs` is a pure Rust TON SDK inspired by `tonutils-go`. It provides
native TON primitives and network clients without depending on third-party Rust
TON SDK crates or native `.so` runtime libraries.

The project is under active development. The current crate is useful for
LiteAPI access over native ADNL TCP, LiteBalancer experiments, TL
serialization, TVM cells and BoC handling, TL-B model work, account and
transaction inspection, smart-contract get-method calls, and scriptable CLI
diagnostics. It is not yet a complete proof-verifying light client, wallet SDK,
ABI layer, DHT/overlay stack, or mempool scanner.

## Current Maturity

Phase 1 closed on 2026-05-09 as the SDK foundation milestone. It added checked
offline fixtures for TVM cells, BoC, dictionaries, TL-B
messages/accounts/transactions, a deterministic upstream-derived TL-B schema
slice for block/config/proof wrappers, LiteClient BoC decode helpers, and
offline CLI inspection.

The active priorities are ergonomic LiteClient, LiteBalancer, contract, wallet,
jetton, and ABI capabilities guided by idiomatic Rust API design and TON
protocol correctness, plus broader fixture evidence. Full trust-level proof
verification, complete `block.tlb` expansion, production balancer behavior,
DHT, overlays, and mempool workflows remain tracked follow-up work in
[TODO.md](TODO.md).

## Where To Start

- New users: read [docs/getting-started.md](docs/getting-started.md), then
  choose a task guide from `docs/`.
- LiteClient callers: start with [docs/liteclient.md](docs/liteclient.md) and
  [docs/balancer.md](docs/balancer.md).
- Contract callers: read [docs/contracts.md](docs/contracts.md) for account
  state and get-method helpers.
- Wallet callers: read [docs/wallets.md](docs/wallets.md) for mnemonic,
  address, and signed transfer helpers.
- TVM and BoC users: read [docs/tvm.md](docs/tvm.md) and
  [docs/tl.md](docs/tl.md).
- CLI users: read [docs/cli.md](docs/cli.md) and
  [docs/examples.md](docs/examples.md).
- Contributors and AI agents: read [AGENTS.md](AGENTS.md),
  [ROADMAP.md](ROADMAP.md), [TODO.md](TODO.md), and
  [dev-docs/README.md](dev-docs/README.md) before changing protocol behavior.

## Quick Commands

Run the minimum local verification:

```bash
cargo check
cargo test --lib
```

Compile examples when examples, feature declarations, or public docs change:

```bash
cargo check --examples --all-features
```

Run common CLI flows with the full feature set:

```bash
cargo run -F full --bin tonutils-rs -- --output json tvm schema check
cargo run -F full --bin tonutils-rs -- --output json tvm boc decode --hex '<boc-hex>' --tlb account
cargo run -F full --bin tonutils-rs -- --output json status
```

Live examples default to public mainnet config download and can be pointed at
testnet or explicit config JSON with the environment variables documented in
[docs/examples.md](docs/examples.md).

## Feature Shape

Default features enable the native network-first path: `std`, `adnl-tcp`, and
`liteclient`. The `liteclient` feature enables TVM and ADNL TCP support, and
TVM/ADNL both enable TL support. `network-config` and `cli` are explicit
features so embedders can avoid config parsing and command-line dependencies
when they only need low-level primitives.

The complete feature map is documented in
[docs/getting-started.md](docs/getting-started.md). Keep new heavyweight
functionality feature-gated and avoid enabling heavy optional dependencies by
default.

## Repository Map

- `src/`: crate code, feature-gated modules, CLI entry points, and tests.
- `docs/`: public user guides organized by task.
- `dev-docs/`: internal TON protocol and implementation reference.
- `examples/`: compile-checked examples for public workflows.
- `fixtures/`: deterministic offline fixture data and metadata.
- `ROADMAP.md`: high-level phases and current project direction.
- `TODO.md`: detailed todo-md task tracker.
- `AGENTS.md`: rules for AI agents and contributors making changes.

## Acknowledgments

This project uses the following references for protocol research, behavior
checks, and API comparisons. Thanks to their authors and maintainers:

- [ton-blockchain/ton](https://github.com/ton-blockchain/ton) by
  [TON Blockchain](https://github.com/ton-blockchain)
- [ton-blockchain/ton4j](https://github.com/ton-blockchain/ton4j) by
  [TON Blockchain](https://github.com/ton-blockchain)
- [xssnick/tonutils-go](https://github.com/xssnick/tonutils-go) by
  [@xssnick](https://github.com/xssnick)
- [tonkeeper/tongo](https://github.com/tonkeeper/tongo) by
  [Tonkeeper](https://github.com/tonkeeper)
- [ston-fi/ton-rs](https://github.com/ston-fi/ton-rs) by
  [STON.fi](https://github.com/ston-fi)
- [ston-fi/tonlib-rs](https://github.com/ston-fi/tonlib-rs) by
  [STON.fi](https://github.com/ston-fi)
- [RSquad/ton-rust-node](https://github.com/RSquad/ton-rust-node) by
  [RSquad](https://github.com/RSquad)
- [nessshon/tonutils](https://github.com/nessshon/tonutils) by
  [@nessshon](https://github.com/nessshon)
- [getgems-io/ton-grpc](https://github.com/getgems-io/ton-grpc) by
  [Getgems](https://github.com/getgems-io)
- [yungwine/ton-mempool](https://github.com/yungwine/ton-mempool) by
  [@yungwine](https://github.com/yungwine)
- [yungwine/pytoniq-core](https://github.com/yungwine/pytoniq-core) by
  [@yungwine](https://github.com/yungwine)
- [yungwine/pytoniq](https://github.com/yungwine/pytoniq) by
  [@yungwine](https://github.com/yungwine)
- [yungwine/pytvm](https://github.com/yungwine/pytvm) by
  [@yungwine](https://github.com/yungwine)
