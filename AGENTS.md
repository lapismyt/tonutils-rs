# AI Agent Handbook

This file is the operating handbook for AI agents and automated contributors
working in `tonutils-rs`. Human-facing project direction lives in
`README.md`, `ROADMAP.md`, `TODO.md`, `docs/`, and `dev-docs/`.

## Rule Hierarchy

Follow these rules in order when instructions conflict:

1. Keep repository text in English: docs, comments, examples, tests, errors,
   TODO entries, and commit-facing notes.
2. Preserve the project direction: a pure Rust TON SDK inspired by
   `tonutils-go`, autonomous and feature-gated.
3. Do not add dependencies on third-party Rust TON SDK crates.
4. Do not introduce native `.so` runtime dependencies.
5. Prefer native Rust implementations for TON-specific logic.
6. Keep heavy or optional functionality behind Cargo features and avoid
   enabling heavy features by default.
7. Use upstream `ton-blockchain/ton` schemas and implementation as the source
   of truth for TL, TL-B, LiteAPI, BoC, and proof behavior.
8. Prefer idiomatic, maintainable, performant Rust APIs and implementation over
   matching pytoniq APIs, naming, module structure, or behavior quirks.
9. Use pytoniq and pytoniq-core only as capability inspiration or comparison
   evidence, never as parity targets or dependency sources.
10. Keep protocol facts in `dev-docs/` before or alongside implementation.
11. Keep `TODO.md` current and todo-md compliant when adding, completing, or
   deferring known gaps.

## Safe Development Workflow

1. Inspect first. Read the relevant Rust modules, tests, `docs/`,
   `dev-docs/`, `ROADMAP.md`, and `TODO.md` before deciding what to change.
2. Preserve user work. The worktree may be dirty; do not revert unrelated
   changes or generated files unless explicitly asked.
3. State assumptions in comments, docs, or final notes when protocol evidence
   is incomplete. Do not hide scope expansion.
4. Update `dev-docs/` before or alongside protocol, serialization, network, or
   trust-model changes.
5. Add or update `TODO.md` entries for known gaps before implementation when
   the gap affects follow-up work.
6. Implement narrowly using existing module patterns and helper APIs.
7. Add focused tests for every protocol, wire-format, serialization, or public
   behavior change.
8. Run at least `cargo check` and `cargo test --lib`. Run broader checks when
   examples, features, CLI behavior, or doctests are affected.
9. Reconcile trackers after implementation: mark completed TODO items, keep
   deferred work visible, and update `ROADMAP.md` only for phase or direction
   changes.

## Protocol Research Rules

- Do not invent TON behavior. Verify constructor names, ids, flags, numeric
  widths, byte order, hash rules, limits, and failure modes from source files,
  checked fixtures, upstream schemas, or recorded live evidence.
- Prefer upstream `ton-blockchain/ton` over SDK behavior when sources disagree.
  Use SDKs such as `tonutils-go`, `tongo`, `pytoniq`, and `pytoniq-core` for
  capability ideas and compatibility comparisons, not as parity targets or
  dependency sources.
- Keep schema maintenance checked and deterministic. Prefer parser or schema
  summary checks over hand-maintained drift.
- When live-network behavior is unavailable, add offline fixtures or mark the
  missing live evidence in `TODO.md`.
- When proof verification is not implemented, document preserved bytes and
  trust assumptions explicitly. Do not describe raw proof payload preservation
  as verified trust.

## Documentation Standards

- Public `docs/` pages should be task-oriented and identify audience,
  prerequisites, feature flags, live-network requirements, examples, current
  limits, and related guides.
- Internal `dev-docs/` pages should describe purpose and scope, wire format or
  data model, invariants and edge cases, crate mapping, tests or fixtures, and
  missing work.
- `dev-docs/README.md` is the table of contents. Update it whenever internal
  docs are added, moved, or removed.
- Prefer precise protocol terms over vague descriptions. Include constructor
  names, flags, numeric limits, byte order, and known failure modes.
- If a topic is not implemented yet, document the intended design and add or
  keep a matching `TODO.md` item.
- Rust doc-comments should explain public module purpose, feature availability,
  important invariants, and whether behavior is stable, partial, or helper-only.
  Add examples only when they are compile-safe or clearly marked `no_run` or
  `ignore`.

## TODO.md Format

`TODO.md` must follow `todo-md/todo-md` conventions:

- Start with `# TODO`.
- Use `##` sections for active task groups.
- Use task lines beginning with `- [ ]`, `- [x]`, or `- [-]`.
- Use indented task lines for subtasks and sub-subtasks.
- Use tags such as `#network`, `#tl`, `#tvm`, `#tests`, `#perf`, and `#docs`.
- Use `# BACKLOG` for postponed work and `# DONE` for completed work.

## Failure Modes

- If tests fail because of your change, fix the change or document the blocker
  before handing off.
- If existing unrelated tests fail, report the exact command and failure
  summary without reverting unrelated work.
- If protocol sources conflict, document the conflict in `dev-docs/`, choose
  the upstream TON behavior unless there is stronger fixture evidence, and add
  a TODO for unresolved compatibility work.
- If fixture evidence is missing, add synthetic tests only for local invariants
  and record the need for upstream or live fixtures.
- If a dependency is needed, justify why a pure Rust crate is materially useful
  and keep it behind the narrowest reasonable feature when it increases cost.

## Reference Sources

Use these sources for protocol research, compatibility checks, and
implementation comparisons. They are references only; do not copy code or add
third-party Rust TON SDK dependencies.

- Upstream TON implementation and schemas: https://github.com/ton-blockchain/ton
- TON JVM SDK reference for API and behavior comparison: https://github.com/ton-blockchain/ton4j
- Official TON documentation index for LLM-assisted research: https://docs.ton.org/llms.txt
- Go SDK reference inspired by this crate's direction: https://github.com/xssnick/tonutils-go
- Tongo SDK reference: https://github.com/tonkeeper/tongo
- STON.fi Rust TON libraries for behavior comparison: https://github.com/ston-fi/ton-rs
- STON.fi tonlib bindings for API behavior comparison: https://github.com/ston-fi/tonlib-rs
- Alternative Rust tonutils implementation for API comparison: https://github.com/nessshon/tonutils
- Getgems TON gRPC reference for API and indexing behavior comparison: https://github.com/getgems-io/ton-grpc
- Mempool research reference: https://github.com/yungwine/ton-mempool
- Pytoniq core primitives reference: https://github.com/yungwine/pytoniq-core
- Pytoniq SDK behavior reference: https://github.com/yungwine/pytoniq
- PyTVM reference implementation: https://github.com/yungwine/pytvm
