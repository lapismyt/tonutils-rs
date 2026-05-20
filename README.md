# tonutils-rs

`tonutils-rs` publishes the `tonutils` crate: a pure Rust TON SDK inspired by
`tonutils-go`. It provides native TON primitives, LiteAPI clients, TVM cells,
BoC, TL, TL-B helpers, wallet utilities, and CLI diagnostics without depending
on third-party Rust TON SDK crates or native `.so` runtime libraries.

The crate is under active development. It is useful for offline TON data
handling, LiteAPI experiments over native ADNL TCP, contract get-method calls,
account and transaction inspection, wallet transfer construction, and
scriptable diagnostics. It is not yet a complete proof-verifying production
light client, DHT/overlay stack, mempool scanner, or full ABI layer.

## Install

Add the crate from crates.io:

```toml
[dependencies]
tonutils = "1.0.0"
```

The default feature set enables the native LiteClient path:

```toml
tonutils = "1.0.0"
```

For offline TVM, BoC, address, dictionary, and TL-B work only:

```toml
tonutils = { version = "1.0.0", default-features = false, features = ["tvm"] }
```

For the CLI and public TON config parsing helpers:

```toml
tonutils = { version = "1.0.0", features = ["network-config", "cli"] }
```

## Feature Flags

- `default`: `std`, `adnl-tcp`, and `liteclient`.
- `tl`: TL types, LiteAPI request and response structures, and serialization.
- `tvm`: cells, slices, builders, BoC, addresses, dictionaries, TL-B helpers,
  and TVM stack values. Enables `tl`.
- `adnl`: ADNL types shared by transports. Enables `tl`.
- `adnl-tcp`: native ADNL TCP transport. Enables `adnl`.
- `liteclient`: LiteAPI client, LiteBalancer, and contract helpers. Enables
  `adnl-tcp` and `tvm`.
- `network-config`: TON global config parsing and liteserver selection.
- `cli`: command-line diagnostics. Enables `liteclient`, `network-config`, and
  JSON ABI helpers.
- `full`: all crate features.

Keep heavyweight or optional integrations behind explicit features when adding
new capabilities.

## Quick Start

Check the crate and examples:

```bash
cargo check
cargo check --examples --all-features
```

Run common CLI diagnostics with the full feature set:

```bash
cargo run -F full --bin tonutils-rs -- --output json status
cargo run -F full --bin tonutils-rs -- --output json tvm schema check
cargo run -F full --bin tonutils-rs -- --output json tvm boc decode --hex '<boc-hex>' --tlb account
```

Run an offline TVM/BoC example:

```bash
cargo run -F tvm --example tvm_boc_roundtrip
```

Run a live LiteClient example using public mainnet config defaults:

```bash
cargo run -F full --example liteclient_masterchain_info
TON_NETWORK=testnet cargo run -F full --example liteclient_masterchain_info
```

Live examples can use `TON_NETWORK`, `TON_GLOBAL_CONFIG_JSON`, `TON_LS_INDEX`,
and task-specific variables documented in [docs/examples.md](docs/examples.md).

## Examples

The repository includes compile-checked examples for:

- LiteClient and LiteBalancer calls over native ADNL TCP.
- Network config loading and liteserver selection.
- Contract account state and get-method helpers.
- TVM cells, BoC, dictionaries, stack values, and addresses.
- TL-B account, message, transaction, block wrapper, and config wrapper
  roundtrips.
- Derive macros for custom TL-B and contract wrappers.
- Offline wallet address derivation and signed transfer BoC construction.

See [docs/examples.md](docs/examples.md) for the full list, feature
requirements, and live-network environment variables.

## Current Capabilities

- Native TL serialization and LiteAPI request/response structures.
- Native ADNL TCP transport and LiteClient request flow.
- LiteBalancer peer management and failover experiments.
- TON cells, slices, builders, BoC, addresses, dictionaries, and TVM stack
  values.
- TL-B models and checked fixture workflows for core account, message,
  transaction, block/config wrapper, and proof-related data.
- Contract account state loading and get-method calls.
- Wallet mnemonic, address, and offline signed transfer helpers.
- Scriptable CLI diagnostics for schema checks, BoC decoding, status checks,
  and live LiteClient workflows.

## Known Limits

- Proof payloads and Merkle proof invariants are preserved and tested in
  focused areas, but full trust-level light-client proof verification is not
  complete.
- DHT, overlays, and mempool workflows are research or follow-up areas, not
  production SDK surfaces.
- LiteBalancer behavior is useful for experiments and diagnostics, but still
  needs broader live-network evidence before being treated as production
  infrastructure.
- The TL-B surface is intentionally growing from checked upstream-derived
  slices and fixtures; unsupported constructors should be treated as explicit
  gaps rather than inferred behavior.
- Live-network examples require network access and should not be run with real
  private keys, seed phrases, or production credentials.

Tracked follow-up work lives in [TODO.md](TODO.md).

## Documentation

HTML documentation is published through GitHub Pages at
<https://lapismyt.github.io/tonutils-rs/>. Build the same site locally with:

```bash
scripts/build-docs-site.sh
```

- [Getting started](docs/getting-started.md): feature selection and guide map.
- [LiteClient](docs/liteclient.md): typed and raw LiteAPI workflows.
- [LiteBalancer](docs/balancer.md): multi-peer workflows and current limits.
- [Contracts](docs/contracts.md): account state and get-method wrappers.
- [Wallets](docs/wallets.md): mnemonic derivation, addresses, and signed
  transfer helpers.
- [TVM primitives](docs/tvm.md): cells, BoC, stack values, addresses, and
  dictionaries.
- [TL and LiteAPI](docs/tl.md): constructors, serialization, and schema checks.
- [Networking](docs/networking.md): ADNL TCP, network config, DHT, and overlay
  boundaries.
- [CLI](docs/cli.md): commands, output formats, and exit behavior.
- [Testing](docs/testing.md): local checks, fixtures, and live-test
  requirements.
- [Internal dev docs](dev-docs/README.md): protocol and implementation notes.

Project direction is tracked in [ROADMAP.md](ROADMAP.md). Contributor and agent
rules are in [AGENTS.md](AGENTS.md).

## Testing

Recommended local checks:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

Use `cargo check --examples --all-features` when examples, feature
declarations, or public docs change. Live tests require explicit environment
variables; see [docs/testing.md](docs/testing.md).

## Contributing

Contributions are welcome when they preserve the project direction: pure Rust
TON implementation work, no third-party Rust TON SDK crate dependencies, no
native `.so` runtime dependencies, feature-gated optional functionality, and
protocol facts grounded in upstream TON sources or checked fixtures.

Read [CONTRIBUTING.md](CONTRIBUTING.md) and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) before opening a pull request.

## Security

Do not open public issues for vulnerabilities. Use GitHub Private Vulnerability
Reporting or a GitHub Security Advisory when possible, and never include real
private keys, seed phrases, production credentials, or non-public user data in
reports. See [SECURITY.md](SECURITY.md).

## Acknowledgments

This project uses these references for protocol research, behavior checks, and
API comparisons. Thanks to their authors and maintainers:

- [ton-blockchain/ton](https://github.com/ton-blockchain/ton)
- [ton-blockchain/ton4j](https://github.com/ton-blockchain/ton4j)
- [xssnick/tonutils-go](https://github.com/xssnick/tonutils-go)
- [tonkeeper/tongo](https://github.com/tonkeeper/tongo)
- [ston-fi/ton-rs](https://github.com/ston-fi/ton-rs)
- [ston-fi/tonlib-rs](https://github.com/ston-fi/tonlib-rs)
- [RSquad/ton-rust-node](https://github.com/RSquad/ton-rust-node)
- [nessshon/tonutils](https://github.com/nessshon/tonutils)
- [getgems-io/ton-grpc](https://github.com/getgems-io/ton-grpc)
- [yungwine/ton-mempool](https://github.com/yungwine/ton-mempool)
- [yungwine/pytoniq-core](https://github.com/yungwine/pytoniq-core)
- [yungwine/pytoniq](https://github.com/yungwine/pytoniq)
- [yungwine/pytvm](https://github.com/yungwine/pytvm)
