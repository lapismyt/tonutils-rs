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
12. Keep repo-tracked files at or below 1000 lines. When a file approaches the
   limit, split by existing module, test, example, or documentation boundaries
   before adding more behavior.

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
7. Do not make new or modified repo-tracked files exceed 1000 lines; if a file
   is already close to the limit, extract a submodule, test module, benchmark,
   example, or docs page first.
8. Add focused tests for every protocol, wire-format, serialization, or public
   behavior change.
9. Before committing, run `cargo fmt`, `cargo check`, `cargo test`, and
   `cargo clippy`. Run broader or narrower focused checks in addition when
   examples, features, CLI behavior, doctests, or protocol-specific surfaces
   are affected. If `prek` is installed and configured, treat its pre-commit
   hooks as an additional verification layer; if hooks fail, fix the agent's
   own errors before handing off.
10. Reconcile trackers after implementation: mark completed TODO items, keep
   deferred work visible, and update `ROADMAP.md` only for phase or direction
   changes.

## Local Agent State

- The local project should have a Git-ignored `.agents/` directory for agent
  notes about local project state.
- The central local note is `.agents/INDEX.md`. If it does not exist, the agent
  must check that the local project meets the minimum local requirements before
  proceeding.
- Minimum local requirements are: `prek` is installed, and `cargo check`,
  `cargo fmt`, `cargo test`, and `cargo clippy` are available.
- Agents may add new Markdown notes under `.agents/` when local state,
  investigation results, or operational details should persist across turns.
  Every new note must be linked from `.agents/INDEX.md`.

## Planning Gate

- For large changes, including public API changes, new features, cross-module
  refactors, protocol behavior changes, or broad documentation rewrites, the
  agent must ask the user for a plan before editing.
- The user may either provide a manually written plan or, when the agent
  supports it, enable planning mode so the agent can draft and confirm the
  plan before implementation.
- Do not start large edits until the plan is available and accepted. If the
  scope becomes large during implementation, pause and request the plan then.

## Branch Workflow

- New features, bug fixes, documentation changes, refactors, tests, and other
  repo changes must start from a dedicated local branch.
- The orchestrating agent creates a new branch from the branch that was active
  before the task began, then treats that new branch as the primary branch for
  the task and any subagent integration.
- If multiple subagents are used, their worktree branches remain intermediate
  local branches and are merged or ported into the orchestrator's task branch.
- After implementation, review, tracker reconciliation, and required checks,
  commit the accepted changes on the task branch.
- After committing the task branch, switch back to the branch that was active
  before the task began and offer to push the completed task branch. Do not
  push without explicit user approval.
- At the end of executing a plan, the final report must include the completed
  task branch name, the new commit hash and summary, and the final current
  branch after any required branch switch.
- If the user later asks to merge a completed feature, fix, or task branch,
  ask whether the local branch should be deleted after the merge. Do not
  delete remote branches unless the user explicitly requests remote cleanup.

## Commit Message Style

- Write commit messages in English.
- Use a short, specific summary that describes the accepted change, not the
  agent process. Avoid placeholders such as `.`, `update`, `fix`, or `wip`.
- Prefer an imperative verb phrase, optionally with a narrow conventional type
  prefix when it improves scanability, for example `docs: update agent workflow
  rules` or `fix: reject invalid cell descriptors`.
- Keep subagent and worktree coordination details out of commit messages unless
  they are directly relevant to the committed project behavior.

## Parallel Agent Workflow

- Prefer parallel development when the orchestrating agent has the required
  permissions and capabilities: creating `git worktree` checkouts, creating and
  merging local Git branches, and spawning subagents.
- Use parallel agents whenever doing so is possible and not unsafe for the
  task. Parallelization is appropriate only when the work can follow the
  rules in this section without risking conflicting edits, unclear ownership,
  leaked secrets, destructive operations, or loss of user work.
- When multiple agents work on the project concurrently, use separate
  `git worktree` checkouts for subagents.
- The orchestrating agent must either create and assign a dedicated
  `git worktree` for each subagent before delegation, or explicitly instruct
  each subagent to create and use its own dedicated `git worktree`.
- Subagents must not modify the primary repository directory that new
  worktrees are created from. Only the orchestrating agent may modify that
  primary directory.
- Keep each subagent's edits confined to its assigned worktree, then merge or
  port the results back through the orchestrating agent.
- After parallel agents finish, the orchestrating agent must inspect the
  changes in each assigned worktree, then merge the accepted changes into the
  primary branch managed by the orchestrator.
- After a subagent's accepted changes are merged or ported into the
  orchestrator task branch, the orchestrator must remove that subagent
  `git worktree` and delete the intermediate local branch.
- At the end of executing a plan, every agent, including orchestrators and
  subagents, must commit its accepted local changes.
- Subagent intermediate branches must remain local and must not be pushed to
  the remote repository.

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
- Tonstack lite-client reference for LiteAPI behavior comparison: https://github.com/tonstack/lite-client
- STON.fi Rust TON libraries for behavior comparison: https://github.com/ston-fi/ton-rs
- STON.fi tonlib bindings for API behavior comparison: https://github.com/ston-fi/tonlib-rs
- RSquad Rust TON node reference for implementation and behavior comparison: https://github.com/RSquad/ton-rust-node
- Alternative Rust tonutils implementation for API comparison: https://github.com/nessshon/tonutils
- Getgems TON gRPC reference for API and indexing behavior comparison: https://github.com/getgems-io/ton-grpc
- Mempool research reference: https://github.com/yungwine/ton-mempool
- Pytoniq core primitives reference: https://github.com/yungwine/pytoniq-core
- Pytoniq SDK behavior reference: https://github.com/yungwine/pytoniq
- PyTVM reference implementation: https://github.com/yungwine/pytvm
