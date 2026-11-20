# Contributing

`tonutils-rs` is a pure-Rust SDK for The Open Network (TON).

## Setup

The workspace targets Rust 1.85 or newer with the stable toolchain. Install the
local tooling with [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall just cargo-deny prek --locked
```

Without `cargo-binstall`, use `cargo install --locked just cargo-deny prek`.

Install the Git hooks once per clone:

```bash
prek install --prepare-hooks
```

The hooks run formatting, clippy, and `cargo-deny` before each commit.

## Checks

```bash
just ci
```

`just ci` runs the same gates as CI. Individual recipes are available:

```bash
just fmt        # format
just fmt-check  # check formatting
just check      # clippy, workspace, all features
just check-cli  # clippy for the CLI
just test       # workspace tests
just schema     # validate checked-in schema metadata
just deny       # licenses, advisories, bans, sources
```

## Branches And Commits

Use a focused topic branch for each change. Use Conventional Commits for pull
request titles, for example:

```text
feat: add wallet transfer helper
fix: reject invalid cell descriptors
docs: clarify LiteClient proof limits
```

Call out feature-flag, protocol, trust-model, and live-network impact in the
pull request.

## Documentation

- `README.md` for the project overview and a minimal example.
- Rustdoc for the public API and item-level invariants.
- `docs/reference/` for maintainer notes on protocols, wire formats, and error
  boundaries.

All code, comments, tests, documentation, and commit messages must be in
English.
