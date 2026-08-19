# Cargo Feature Matrix

The crate should compile in small configurations. Optional features must isolate dependencies that are not needed by core users.

## Intended Features

| Feature | Purpose | Expected modules |
| --- | --- | --- |
| `std` | Standard library support | all default builds |
| `tl` | TL types and helpers | `src/tl` |
| `tvm` | TVM primitives | `src/tvm`, `src/tlb` |
| `adnl` | ADNL crypto and base types | `src/adnl` without TCP runtime if split further |
| `adnl-tcp` | async TCP ADNL | ADNL peer, codec, handshake over Tokio |
| `liteclient` | LiteAPI client | `src/liteclient` |
| `network-config` | TON global config parsing | `src/network_config` |
| `cli` | command line app | `src/cli`, `src/main.rs` |

Default target:

```toml
default = ["std", "adnl-tcp", "liteclient"]
```

## Dependency Policy

Always acceptable when needed:

- pure Rust crypto crates,
- pure Rust async crates,
- serialization crates,
- testing and fixture crates as dev-dependencies.

Avoid:

- native runtime libraries,
- third-party Rust TON SDK crates,
- mandatory HTTP clients in library default features,
- mandatory logging implementations.

## Verification Matrix

Required local commands:

```bash
cargo check --no-default-features
cargo check
cargo check --all-features
cargo test
cargo test --all-features
```

CI should also run:

```bash
cargo fmt --check
```

## Feature Design Rules

- A module behind a feature should not leak types into always-compiled public APIs.
- Optional dependencies should be marked with `dep:name` in feature definitions.
- Dev-dependencies are allowed for tests even if the corresponding runtime dependency is optional.
- Examples and binaries should use `required-features`.

## Known Gaps

- `tl` currently has response conversion helpers that mention `LiteError`.
- `adnl` and `adnl-tcp` need a cleaner split if UDP support is added.
- `network-config` parsing and HTTP downloading should remain separate.
