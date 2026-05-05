# Repository Guidelines

## Project Direction

`tonutils` is a pure Rust TON SDK inspired by `tonutils-go`. The library should stay autonomous, flexible, and feature-gated. Native Rust implementations are preferred for TON-specific logic. Do not add dependencies on third-party Rust TON SDK crates.

## Language

All repository text must be English: documentation, comments, errors, examples, tests, TODO entries, and commit-facing notes.

## Dependency Policy

- Do not introduce native `.so` runtime dependencies.
- Pure Rust crypto, async, parsing, and performance-oriented crates are acceptable when they materially improve correctness or throughput.
- Keep optional functionality behind Cargo features when it increases dependency weight or runtime cost.
- Avoid enabling heavy features by default.

## Development Flow

1. Keep protocol facts in `dev-docs/` before or alongside implementation.
2. Treat `dev-docs/` as the repository's internal TON technical documentation, not as brief implementation notes. Each file should explain the relevant TON concepts, wire formats, invariants, edge cases, and how this crate maps them into Rust.
3. Keep `TODO.md` current when adding known gaps.
4. `TODO.md` must follow the `todo-md/todo-md` format:
   - Start with `# TODO`.
   - Use `##` sections for active task groups.
   - Use task lines beginning with `- [ ]`, `- [x]`, or `- [-]`.
   - Use indented task lines for subtasks and sub-subtasks.
   - Use tags such as `#network`, `#tl`, `#tvm`, `#tests`, `#perf`, and `#docs`.
   - Use `# BACKLOG` for postponed work and `# DONE` for completed work.
5. Preserve existing tests and add focused coverage for every protocol or serialization change.
6. Prefer checked schema-driven TL maintenance over hand-maintained drift.
7. Use upstream TON schemas from `ton-blockchain/ton` as the source of truth.

### Feature Change Workflow

When adding or changing SDK functionality, follow this sequence:

1. Update `dev-docs/` before or alongside implementation with the technical context needed to understand the change.
2. Add missing `TODO.md` items for known gaps or follow-up work before implementation, using the format rules above.
3. Implement the code and documentation changes.
4. Run the relevant tests. `cargo check` and `cargo test --lib` are the minimum unless the change clearly requires broader verification.
5. Reconcile `TODO.md` after implementation by marking completed items, adding newly discovered gaps, or moving items as needed.
6. Update `ROADMAP.md` when the change affects roadmap status, phase scope, or project direction.

## Reference Sources

Use these sources for protocol research, compatibility checks, and implementation comparisons. They are references only; do not copy code or introduce dependencies on third-party Rust TON SDK crates.

- Upstream TON implementation and schemas: https://github.com/ton-blockchain/ton
- Official TON documentation index for LLM-assisted research: https://docs.ton.org/llms.txt
- Go SDK reference inspired by this crate's direction: https://github.com/xssnick/tonutils-go
- Tongo SDK reference: https://github.com/tonkeeper/tongo
- STON.fi Rust TON libraries for behavior comparison: https://github.com/ston-fi/ton-rs
- STON.fi tonlib bindings for API behavior comparison: https://github.com/ston-fi/tonlib-rs
- Alternative Rust tonutils implementation for API comparison: https://github.com/nessshon/tonutils
- Mempool research reference: https://github.com/yungwine/ton-mempool
- Pytoniq core primitives reference: https://github.com/yungwine/pytoniq-core
- Pytoniq SDK behavior reference: https://github.com/yungwine/pytoniq
- PyTVM reference implementation: https://github.com/yungwine/pytvm

## Documentation Rules

- Write repository documentation in English only.
- Prefer precise protocol terms over vague descriptions.
- Include numeric limits, byte order, constructor names, flags, and known failure modes when documenting protocol behavior.
- When a topic is not implemented yet, document the intended design and mark missing work in `TODO.md`.
- Keep `dev-docs/` structured by TON subsystem:
  - `architecture/`: crate design, feature gates, dependency policy, errors.
  - `tl/`: TL syntax, schema maintenance, LiteAPI wire definitions.
  - `network/`: ADNL TCP/UDP, DHT, overlays, liteserver config.
  - `tvm/`: cells, BoC, slices, builders, dictionaries, TLB, stack values.
  - `liteclient/`: request flow, balancing, proof verification and trust model.
  - `contracts/`: get-methods, external messages, high-level contract wrappers.
  - `research/`: mempool scanning and protocol investigations.
  - `testing/`: fixtures, live tests, benchmarking.
- Every `dev-docs` topic file should include:
  - purpose and scope,
  - wire format or data model,
  - invariants and edge cases,
  - current crate mapping,
  - missing work.
- `dev-docs/README.md` is the table of contents and must be updated whenever files are added, moved, or removed.

## Verification

Run at least:

```bash
cargo check
cargo test --lib
```

Run full `cargo test` when examples and optional targets are expected to compile.
