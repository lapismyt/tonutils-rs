# Contributing

Thanks for improving `tonutils-rs`. The project aims to be a pure Rust TON SDK
with native protocol implementations, feature-gated optional functionality, no
third-party Rust TON SDK crate dependencies, and no native `.so` runtime
dependencies.

## Local Setup

Install Rust with `rustup` before selecting a toolchain. On Unix-like systems,
including Linux, macOS, and WSL, run the official installer and follow the
prompts:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, use the official installer from
<https://www.rust-lang.org/tools/install>. Reopen the shell if needed so the
`~/.cargo/bin` or `%USERPROFILE%\.cargo\bin` PATH update is visible, then
verify the Rust tools:

```bash
rustc --version
cargo --version
```

Install and select the stable Rust toolchain:

```bash
rustup toolchain install stable
rustup default stable
```

Rustup provides the `cargo` command, Rust compiler, formatter, test runner, and
Clippy checks used by this crate.

Install `prek` so you can run the configured pre-commit quality hooks locally
before commits and pull requests. The preferred path is the standalone installer
from the `j178/prek` releases:

```bash
# Linux / macOS
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/j178/prek/releases/download/v0.4.1/prek-installer.sh | sh

# Windows (PowerShell)
powershell -ExecutionPolicy ByPass -c "irm https://github.com/j178/prek/releases/download/v0.4.1/prek-installer.ps1 | iex"
```

If you already have a recent Rust toolchain, you can alternatively build `prek`
from crates.io. Current upstream `prek` requires Rust 1.89 or newer for this
path:

```bash
cargo install --locked prek
```

CodeGraph is only needed when developing with agents. It gives agents a fast
symbol and call-graph index so they can inspect Rust definitions, callers, and
file structure without repeatedly scanning the checkout. Agent users need both
the MCP server configuration and the CLI binaries.

Configure the CodeGraph MCP server with:

```bash
npx @colbymchenry/codegraph
```

If this fails with `npx: command not found`, install Node.js and npm first,
then reopen the shell and verify:

```bash
node --version
npm --version
npx --version
```

If `npm` is available but `npx` is not, use `npm exec -- @colbymchenry/codegraph`
or update the npm/Node.js installation. Modern `npx` is backed by `npm exec`,
so this uses the same package execution path.

Install the CodeGraph CLI binaries with the portable installer:

```bash
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/colbymchenry/codegraph/main/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/colbymchenry/codegraph/main/install.ps1 | iex
```

If you do not use the portable installer, install the npm-provided CLI binary:

```bash
npm i -g @colbymchenry/codegraph
```

After the MCP server and CLI binary are available, initialize the index from
the repository checkout:

```bash
codegraph init -i
```

Verify the checkout:

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
