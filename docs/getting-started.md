# Getting Started

`tonutils` is a native Rust TON SDK. The current public surface is focused on
LiteAPI access over native ADNL TCP, TL serialization, TON cells/BoC primitives,
network config parsing, and a scriptable CLI.

## Feature Selection

Default features enable the native LiteClient path over ADNL TCP:

```toml
tonutils = { path = "../tonutils-rs" }
```

The default feature set is `std`, `adnl-tcp`, and `liteclient`. Because
`liteclient` depends on `tvm`, `adnl-tcp` depends on `adnl`, and both `tvm` and
`adnl` depend on `tl`, the default build also compiles TL and TVM support.

Use narrower features when embedding only a part of the SDK:

```toml
tonutils = { path = "../tonutils-rs", default-features = false, features = ["tvm"] }
```

Enable config parsing and CLI support explicitly:

```toml
tonutils = { path = "../tonutils-rs", features = ["network-config", "cli"] }
```

## Current Feature Map

- `std`: standard library support. It is currently part of the default build.
- `tl`: TL types, LiteAPI request and response structures, and serialization
  helpers.
- `tvm`: cells, slices, builders, BoC, addresses, dictionaries, TL-B helpers,
  and TVM stack values. Enables `tl`.
- `adnl`: ADNL types shared by transports. Enables `tl`.
- `adnl-tcp`: native ADNL TCP transport. Enables `adnl` plus async transport
  dependencies.
- `liteclient`: LiteAPI client, LiteBalancer, and contract helpers over ADNL
  TCP. Enables `adnl-tcp` and `tvm`.
- `network-config`: TON global config parsing and liteserver selection helpers.
- `cli`: command line interface for shell scripts and diagnostics. Enables
  `liteclient` and `network-config`.

Future feature groups may add proof verification, wallets, DHT, overlays,
mempool scanning, and optional TON emulator bindings.

## Guide Map

- [LiteClient](liteclient.md): typed and raw LiteAPI workflows.
- [LiteBalancer](balancer.md): multi-peer workflows and prototype limits.
- [Contracts](contracts.md): account state and get-method wrappers.
- [TVM primitives](tvm.md): cells, BoC, stack values, addresses, and dictionaries.
- [TL and LiteAPI](tl.md): constructors, serialization, raw bytes, and schema checks.
- [Networking](networking.md): ADNL TCP, network config, and future protocol boundaries.
- [CLI](cli.md): shell commands, output formats, and exit behavior.
- [Examples](examples.md): compiling examples and live input variables.
- [Testing](testing.md): local checks, live tests, and fixture expectations.
