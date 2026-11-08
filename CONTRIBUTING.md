# Contributing

`tonutils-rs` is a pure-Rust SDK for The Open Network

## Documentation Layout

The maintained documentation surface is intentionally small:

- `README.md` for the project overview, quickstart, feature matrix.
- Rustdoc for the public API and item-level invariants.
- `docs/reference/` for maintainer notes, protocol ownership, wire formats,
  error boundaries, and schema workflow.

`TODO.md` tracks deferred work and known gaps. Do not add a separate user-guide
site or recreate removed guide pages.

## Toolchain

The workspace targets Rust 1.85 or newer and uses the stable toolchain:

```bash
rustup toolchain install stable
rustup default stable
rustc --version
cargo --version
```

All code, comments, tests, documentation, and commit messages must be in
English.

## Design Constraints

- Prefer explicit imports and readable code. Do not add function-local `use`
  statements, glob imports, or `use super::*`.
- Add focused tests for protocol, wire-format, serialization, and error-boundary
  changes.

## Verification

Run formatting, linting, tests, schema validation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p tonutils-schema-gen -- check
```

Check the individual public features when changing feature gates, imports, error types:

```bash
for feature in adnl tl tvm tlb network-config metadata liteclient contracts wallet crc serde; do
  cargo check -p tonutils --no-default-features --features "$feature"
done
```

## Branches, Commits, And Versions

Use a focused topic branch for each change. Use Conventional Commits for pull requests names, for
example:

```text
feat: add wallet transfer helper
fix: reject invalid cell descriptors
docs: clarify LiteClient proof limits
```

You can use any commit names inside of pull request (branch). Call out feature-flag,
protocol, trust-model, and live-network impact in the pull request.
