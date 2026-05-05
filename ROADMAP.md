# Roadmap

This roadmap describes the intended development phases for `tonutils`. It is a
high-level planning document: `TODO.md` remains the detailed task tracker, and
`dev-docs/README.md` is the entry point for protocol and implementation notes.

## Direction

`tonutils` is a pure Rust TON SDK inspired by `tonutils-go`. The crate should
stay autonomous, flexible, and feature-gated.

Core constraints:

- Implement TON-specific logic natively in this repository.
- Do not depend on third-party Rust TON SDK crates.
- Do not introduce native `.so` runtime dependencies.
- Keep heavy optional functionality behind Cargo features.
- Preserve low-level protocol access while building ergonomic high-level APIs.

## Current Status

The project has a strong foundation (feature gates, ADNL TCP transport,
LiteClient/LiteBalancer surfaces, TVM primitives, contract wrappers, CLI, and
dev-docs). These are enablers, not the top priority.

Immediate priority is now:

1. Behavioral parity with `pytoniq` for the full user-facing SDK scope.
2. A full ABI stack comparable to `tongo` (types, encode/decode, JSON ABI,
   contract integration, and fixture-backed compatibility tests).

Hardening, productionization, and broad protocol expansion remain important but
are intentionally deferred until parity and ABI milestones are complete.

## Phase 1: Pytoniq Behavioral Parity Program

Deliver pytoniq-equivalent behavior (not 1:1 internal architecture) across the
full user-facing surface:

- TVM and schema primitives expected by pytoniq users: `TlbScheme`, `Cell`,
  `Slice`, `Builder`, and related serialization and decoding behavior.
- Full LiteClient method parity: all pytoniq-exposed LiteClient methods and
  their expected request/response and error behavior.
- Full LiteBalancer method parity: all pytoniq-exposed balancer methods and
  expected failover/request semantics.
- RPS control beyond pytoniq parity: configurable per-peer and global request
  rate limiting suitable for rented liteservers (including tonconsole-style
  quotas), with predictable throttling behavior.
- Contract and wallet flows: `Contract` wrappers, wallet operations, and
  end-to-end call/message workflows expected by pytoniq users.
- Mnemonic workflows: generation, import/export, seed/key derivation, and
  validation behavior compatible with pytoniq expectations.
- Networking behavior expected by pytoniq-style usage: timeout defaults,
  retries/failover semantics, and error surfaces that are predictable in
  scripts and integrations.
- Build and maintain a parity matrix that maps each pytoniq-facing workflow to
  status, known deviations, tests, and planned closure.
- Add fixture-backed and ignored live-network compatibility tests for parity
  acceptance criteria.

Exit criteria for Phase 1:

- Documented parity acceptance criteria for the full pytoniq-facing surface.
- Compatibility matrix with explicit pass/fail status and tracked gaps.
- Reproducible tests covering successful and failure-mode behavior.
- Verified RPS limiting behavior for tonconsole-style rented liteserver usage.

Current status: local per-peer and global RPS throttling is implemented for
`LiteClient`, `LiteBalancer`, and CLI workflows. Live tonconsole-style rented
liteserver validation remains open until credentials or a test endpoint are
available.

## Phase 2: Full ABI Stack (Tongo-Level Capability)

Implement a full ABI subsystem comparable in scope to tongo:

- ABI data model: contracts, functions, events, inputs/outputs, tuples,
  optional fields, arrays, dictionaries, and TVM-relevant scalar types.
- Encoding and decoding engine: ABI value to TVM stack/cell/message payload and
  inverse decoding for method outputs and event payloads.
- JSON ABI parser and loader with schema validation and precise diagnostics.
- Contract integration: get-method argument encoding and external message body
  construction driven by ABI definitions.
- Fixture-backed golden tests and cross-reference validation against known
  tongo/TON behavior.

Exit criteria for Phase 2:

- Stable ABI module surface for parse, encode, decode, and integration.
- Golden fixtures and compatibility tests for representative real-world ABI
  contracts and edge cases.
- Clear documentation of supported ABI scope and known limitations.

## Phase 3: Hardening, Reliability, And Productionization

After parity and ABI milestones:

- Harden ADNL TCP behavior around boundaries, timeouts, graceful close, and
  structured diagnostics.
- Replace prototype balancer behavior with explicit peer states, reconnects,
  backoff, scoring, and clean shutdown.
- Stabilize CLI behavior and machine-readable outputs across supported commands.
- Make TVM cell, BoC, slice, builder, dictionary, and stack behavior fully
  spec-accurate with expanded golden fixtures.
- Add proof verification models and trust documentation for light client usage.

## Phase 4: Performance, Extended Protocols, And Ecosystem Coverage

After production hardening:

- Add benchmarks and allocation audits for ADNL, TL, TVM, BoC, and balancer
  hot paths.
- Implement ADNL UDP, DHT, overlay, and mempool scanning APIs with captured
  fixtures and later live-network tests.
- Expand docs/examples coverage to match the finalized high-level APIs and
  CLI workflows.

## Later Backlog

These items remain intentionally postponed:

- Toncenter-compatible HTTP API client.
- WASM and no-std feasibility audits.
- Wallet contract builders.
- Jetton and NFT convenience packages.
- Storage daemon protocol support.
- Validator engine control API support.

## Roadmap Maintenance

Update this file when project direction or major phases change. Keep detailed
implementation tasks in `TODO.md`, and keep protocol facts, wire formats,
invariants, and source-tracking notes in `dev-docs/`.
