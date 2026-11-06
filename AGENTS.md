# AGENTS.md - Operating Handbook for AI Agents

## 1. Critical Constraints
- **English Only:** All code, docs, comments, tests, commits.
- **Pure Rust TON SDK:** NO third-party TON crates. Everything TON-related must be implemented here.
- **Native Libs:** Avoid requiring end-users to install `.so`/`.dll` at runtime. Static linking or bundled binaries are preferred, but runtime dependencies are allowed for specific tasks if strictly justified.
- **Upstream Truth:** `ton-blockchain/ton` is the source of truth for TL/TL-B/LiteAPI/BoC/proofs. Never invent TON behavior.
- **Idiomatic Rust:** Standard patterns > matching other SDKs' quirks. Copy existing project style.

## 2. Rust & Code Standards
- **Errors:** Use `thiserror` for lib, `anyhow` for bins/tests. 
- **Panics:** `.unwrap()`/`.expect()` are allowed ONLY if logically guaranteed to succeed (e.g., parsing known-valid internal data). MUST include a comment explaining *why* it cannot fail.
- **Unsafe:** Banned unless accompanied by `// SAFETY:` explaining invariants.
- **Readability >= Performance:** Readable code is easier to optimize. Optimize algorithms first, not syntax. Do not sacrifice readability for micro-optimizations.
- **No Stubs:** NEVER leave `todo!()`, `unimplemented!()`, or dummy code unless explicitly requested by the user.
- **Async:** Use `tokio`. NO blocking I/O in async contexts.
- **Size:** ~1000 lines/file max. Simplify first, split second. NO readability loss for line count.
- **Anti-patterns:** NO `use` inside functions. NO `use super::*`. NO `Box<dyn Trait>` without reason. NO unused private fields or dead code.

## 3. Execution Workflow
### Before Coding
1. **Read context:** Relevant modules, tests, `docs/`, `docs/reference/` (internal docs are crucial for AI context), and `TODO.md`.
2. **Plan:** Ask user for a plan on large changes (public API, cross-module, protocol).
3. **Think:** Use `<thinking>` tags to reason about constraints before writing code.

### During Coding
- Implement narrowly using existing patterns.
- Add focused tests for protocol/wire/serialization changes.
- Update `docs/reference/` alongside protocol changes.

### Before Committing (Definition of Done)
- [ ] `cargo clippy` - clean (covers `cargo check`)
- [ ] `cargo test` - all pass
- [ ] `cargo fmt` - formatted
- [ ] Tests added for new behavior
- [ ] Docs updated if needed

## 4. Git & Versioning
- **Branches:** Dedicated branch per task. NO pushing without explicit approval. Report final branch & commit hash.
- **Commits:** Conventional Commits (English, no placeholders like `.` or `wip`). Format: `type(scope): description`.
- **Versioning:** SemVer 2.0.0. Offer to bump version/`CHANGELOG.md` if public behavior changes.
- **Parallel Agents:** Use `git worktree` for subagents. Merge back, then cleanup worktrees/branches.

## 5. Local State (Persistent Memory)
- **Why it exists:** `.agents/` acts as persistent memory across sessions. It prevents the agent from re-discovering project quirks, losing context, or repeating mistakes.
- **Structure:** Git-ignored `.agents/` directory. Central note is `.agents/INDEX.md` (create if missing after verifying `cargo clippy/test/fmt` work).
- **Usage:** Add new Markdown notes under `.agents/` for local state or investigation results. Every new note MUST be linked from `INDEX.md`.

## 6. Documentation Standards
- **Public `docs/`:** Task-oriented (audience, prerequisites, examples, limits).
- **Internal `docs/reference/`:** Wire formats, invariants, edge cases, crate mapping. Keep `README.md` TOC updated. Use precise protocol terms.
- **`TODO.md`:** Keep minimal. Move deep context to `docs/reference/` and reference it.
- **Rust Docs:** Explain purpose, feature availability, invariants. Examples must be compile-safe or marked `no_run`/`ignore`.

## 7. Reference Sources (For Comparison Only, NO Code Copying)
*Use for protocol research and behavior comparison. Do not add as dependencies.*

- **C++ (Upstream):** `ton-blockchain/ton`
- **Go:** `xssnick/tonutils-go`, `xssnick/gton`, `tonkeeper/tongo`, `tonstack/lite-client`
- **Rust:** `ston-fi/ton-rs`, `ston-fi/tonlib-rs`, `RSquad/ton-rust-node`, `nessshon/tonutils`, `getgems-io/ton-grpc`, `yungwine/ton-mempool`
- **Java:** `ton-blockchain/ton4j`
- **Python:** `yungwine/pytoniq`, `yungwine/pytoniq-core`, `yungwine/pytvm`
- **Docs:** `docs.ton.org/llms.txt`

## 8. AI Execution Hacks
- **Self-Correction:** Review generated code against "Critical Constraints" before finalizing.
- **Output Rule:** NO conversational filler. Output ONLY code, diffs, or terminal commands.
