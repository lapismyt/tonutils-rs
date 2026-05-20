# Contributing

Thanks for improving `tonutils-rs`. The project aims to be a pure Rust TON SDK
with native protocol implementations, feature-gated optional functionality, no
third-party Rust TON SDK crate dependencies, and no native `.so` runtime
dependencies.

## Local Setup

Install the stable Rust toolchain and `prek`, then verify the checkout:

```bash
cargo fmt --check
cargo check
cargo test
cargo clippy -- -D warnings
```

Run narrower checks while developing when they cover the changed surface. Before
opening a pull request, run the full relevant checks, and add
`cargo check --examples --all-features` when examples, feature declarations, or
public docs change.

## Branches And Pull Requests

Create a focused topic branch for each change. Pull requests should describe
the user-visible behavior, link related issues when applicable, and call out any
feature flag, public API, protocol, trust-model, or live-network impact.

Protocol, serialization, network, trust-model, and public API changes should
include focused tests and relevant updates to `docs/`, `dev-docs/`, or
`TODO.md`.

## Protocol Research

Do not invent TON behavior. Verify constructor names, ids, flags, numeric
widths, byte order, hash rules, limits, and failure modes from upstream TON
schemas and implementation, checked fixtures, or recorded live evidence.

Prefer upstream `ton-blockchain/ton` when references disagree. Use other SDKs
only for capability ideas and compatibility comparison, not as dependency
sources or parity targets. If evidence is incomplete, document the assumption
and keep the gap visible in `TODO.md`.

## Documentation And TODOs

Keep repository text in English. Update public docs for user-facing behavior,
internal `dev-docs/` for protocol or implementation details, and `TODO.md` for
known gaps, deferred work, or missing fixture evidence.

`TODO.md` follows `todo-md/todo-md` conventions: task lines begin with `- [ ]`,
`- [x]`, or `- [-]`, active groups use `##` headings, and postponed/completed
work belongs under `# BACKLOG` or `# DONE`.

## Commits And Versions

Use Conventional Commits 1.0.0 summaries, such as:

```text
feat: add wallet transfer helper
fix: reject invalid cell descriptors
docs: clarify LiteClient proof limits
```

The crate follows SemVer 2.0.0. Public API additions, deprecations, removals,
or behavior changes may require a version update. Security fixes and release
workflow changes should call out the expected version impact.

## Live Tests And Secrets

Live-network tests and examples are opt-in and require explicit environment
variables documented in [docs/testing.md](docs/testing.md) and
[docs/examples.md](docs/examples.md).

Never commit real private keys, seed phrases, live credentials, production
configuration, or non-public user data. Use deterministic offline fixtures when
possible, and sanitize live evidence before adding it to the repository.
