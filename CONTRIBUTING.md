# Contributing

`tonutils-rs` is a pure-Rust SDK for The Open Network. Contributions must keep
TON behavior native to this workspace: do not add a third-party TON SDK crate,
and do not require users to install a native `.so` or `.dll` at runtime.

## Documentation Layout

The maintained documentation surface is intentionally small:

- `README.md` for the project overview, quickstart, feature matrix, and trust
  boundary.
- Rustdoc for the public API and item-level invariants.
- `docs/reference/` for maintainer notes, protocol ownership, wire formats,
  error boundaries, and schema workflow.
- `CONTRIBUTING.md` for development and verification instructions.

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

- Use `thiserror` for library errors. `anyhow` is allowed only in tests and
  binaries; it must not appear in `tonutils`'s normal dependency graph or public
  API.
- Do not leave stubs, `todo!()`, `unimplemented!()`, or dummy implementations.
- Avoid `unsafe`. If it is unavoidable, document the invariants with a
  `// SAFETY:` comment.
- Keep async I/O on Tokio. Do not block a Tokio worker with synchronous file or
  socket I/O.
- Prefer explicit imports and readable code. Do not add function-local `use`
  statements, glob imports, or `use super::*`.
- Add focused tests for protocol, wire-format, serialization, and error-boundary
  changes.

## Verification

Run formatting, linting, tests, schema validation, and rustdoc before opening a
pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p tonutils-schema-gen -- check
RUSTDOCFLAGS="-D warnings" cargo doc -p tonutils --all-features --no-deps
```

Verify that the normal library dependency graph has no `anyhow`:

```bash
cargo tree -p tonutils -e normal
```

Check the individual public features when changing feature gates, imports, or
error types:

```bash
for feature in adnl tl tvm tlb network-config metadata liteclient contracts wallet overlay overlay-config mempool crc serde; do
  cargo check -p tonutils --no-default-features --features "$feature"
done
```

Run `cargo fmt --all` to apply formatting. If a check fails for a reason
unrelated to your change, leave it visible in the pull request rather than
silently changing unrelated code.

## Schema Workflow

Checked-in `.tl` and `.tlb` snapshots are protocol evidence, not generated
scratch files. The generator validates constructor metadata and the committed
inventory.

When a schema intentionally changes:

1. Update the snapshot under `crates/tonutils/src/tl/schemas/` or
   `crates/tonutils/src/tlb/schemas/` from an upstream-backed source.
2. Update the handwritten wire types that consume it.
3. Regenerate metadata and inventory with:

   ```bash
   cargo run -p tonutils-schema-gen -- --write
   ```

4. Run `cargo run -p tonutils-schema-gen -- check` and commit the snapshot,
   generated files, and `docs/reference/schema-inventory.tsv` together.

Generated constructor metadata names are schema-specific:
`LiteApiConstructorMetadata`, `TonApiConstructorMetadata`,
`TonlibApiConstructorMetadata`, and `BlockTlbConstructorMetadata`.

## Protocol Research

Do not invent TON behavior. Verify constructor names and ids, field widths,
flags, byte order, hash rules, cell limits, and failure modes against upstream
`ton-blockchain/ton`, checked fixtures, or recorded live evidence. Prefer
upstream when references disagree. Other SDKs are comparison material only and
must not become dependencies.

Protocol ownership and wire invariants live in `docs/reference/README.md`.
Update that file when moving a wire type between modules or changing an error
boundary. If evidence is incomplete, document the assumption and keep the gap
visible in `TODO.md`.

## Tests And Live Networks

Deterministic offline tests are the default. Live liteserver, ADNL, overlay, and
mempool tests are opt-in and must not run during a routine offline test pass.
Their required environment variables are documented in the live-test workflow
and in the relevant ignored test's module-level documentation.

Never commit private keys, seed phrases, live credentials, production
configuration, or non-public user data. Sanitize captured live evidence and
prefer checked offline fixtures.

## Branches, Commits, And Versions

Use a focused topic branch for each change. Use Conventional Commits, for
example:

```text
feat: add wallet transfer helper
fix: reject invalid cell descriptors
docs: clarify LiteClient proof limits
```

Do not use placeholder subjects such as `.` or `wip`. Public API changes,
behavior changes, and deprecations follow SemVer 2.0.0. Call out feature-flag,
protocol, trust-model, and live-network impact in the pull request. Update
`TODO.md` only when the described work is actually complete and verified.
