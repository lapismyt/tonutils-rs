# Roadmap

This roadmap describes the intended development phases for `tonutils-rs`. It
is a high-level planning document: `TODO.md` is the detailed task tracker, and
`docs/reference/README.md` is the entry point for protocol evidence and
implementation notes.

## Direction

`tonutils-rs` is a pure Rust TON SDK inspired by `tonutils-go`. It remains
autonomous, flexible, and feature-gated.

Core constraints:

- Implement TON-specific logic natively in this repository.
- Do not depend on third-party Rust TON SDK crates.
- Do not introduce native `.so` runtime dependencies.
- Keep heavy optional functionality behind Cargo features.
- Preserve low-level protocol access while building ergonomic high-level APIs.

## Current Status

The project has completed its transition to the modular `tonutils-*` workspace
for the upcoming 2.0 release. The independent crates separate TVM, TL, TL-B,
ADNL, LiteClient, contracts, wallet, metadata, and related concerns while
sharing a workspace version and feature-aware validation.

The foundation already includes native TVM and BoC primitives, ADNL TCP,
LiteClient and LiteBalancer surfaces, typed-stack contract helpers, contract
blueprints and provider boundaries, Wallet V4R2/V5R1 support, jetton and NFT
payloads, and raw-preserving TEP metadata parsing. These are established
capabilities, not the immediate roadmap bottleneck. Their remaining gaps stay
explicitly tracked in `TODO.md`.

The current priority order is:

1. Complete deterministic, typed coverage of the pinned upstream TL and TL-B
   schemas without treating parser gaps as opaque success paths.
2. Replace raw-preserving block, shard-state, configuration, and proof wrappers
   with stable typed models, backed by checked upstream or live-captured
   fixtures and documented trust assumptions.
3. Harden LiteBalancer and ADNL TCP reliability: connection lifecycle,
   reconnects, backoff, peer scoring, boundary handling, and fixture-backed or
   live-network evidence.

## Initial compatibility: TVM, BoC, TL, And TL-B Foundation

Status: closed on 2026-05-09 as the compatibility foundation milestone.

Initial compatibility delivered the low-level primitives needed to encode, decode, and model
TON data without third-party Rust TON SDK crates. It includes cells, slices,
builders, BoC handling, addresses, dictionaries, TVM stack values, TL and TL-B
schema support, deterministic schema inventory checks, and an initial typed
block/config/proof wrapper slice.

The closed milestone does not imply that all upstream schemas or protocol
fixtures are complete. The complete typed upstream TL/TL-B migration, deep
block/config/proof models, and additional independent fixture evidence remain
active work in `TODO.md`.

## High-level SDK: High-Level Contracts And Wallets

Status: substantially complete; follow-up validation and coverage remain open.

High-level SDK built ergonomic SDK surfaces on the low-level foundation:

- Provider-bound contract clients, contract blueprints, state-init address
  derivation, account code/data/balance access, and typed get-method stack
  conversions.
- Wallet V5R1 and V4R2 mnemonic, state, signed-body, external-message, and
  provider workflows with deterministic offline coverage.
- Typed jetton and NFT payload helpers plus common TEP-64 metadata parsing that
  preserves unsupported or malformed content as raw cells.
- LiteClient and LiteBalancer helpers for contract-facing account, method,
  transaction, block, and configuration workflows.

The remaining work is intentionally not hidden by this phase label: expand
protocol-backed fixtures and live evidence, complete typed response coverage,
and keep helper behavior synchronized as the TL/TL-B model surface grows. See
`TODO.md` for the implementation-level checklist.

## Phase 3: Protocol Coverage And Reliability

The next phase is driven by protocol completeness and evidence before broader
surface expansion:

- Generate or handwrite complete typed models for the pinned upstream TL and
  TL-B schema sets, with deterministic source inventory and drift checks.
- Complete typed `block.tlb` coverage for blocks, shard states, configuration
  parameters, Merkle proofs, and Merkle updates; preserve raw bytes only where
  a typed interpretation is explicitly unavailable.
- Add checked upstream or live-captured fixtures for representative account,
  block, configuration, shard-state, and proof payloads. Clearly distinguish
  structural inspection from trustless proof verification.
- Harden ADNL TCP around frame limits, handshake failures, timeouts, graceful
  close, and diagnostics.
- Make LiteBalancer peer lifecycle explicit and reliable: reconnect descriptors,
  backoff, health transitions, scoring, routing, and clean shutdown.

## Phase 4: Performance, Extended Protocols, And Ecosystem Coverage

The initial `tonutils-overlay` and `tonutils-mempool` primitives now provide
offline-safe bounded peer/event handling and a Rust stream for deduplicated
pending external messages. Live DHT bootstrap, canonical overlay wire schemas,
and LiteServer inclusion tracking remain protocol-evidence work in `TODO.md`.

After protocol coverage and reliability are established:

- Add benchmarks and allocation audits for ADNL, TL, TVM, BoC, and balancer
  hot paths.
- Implement native Rust ADNL UDP, DHT, overlay, and QUIC transports with
  captured fixtures and later live-network tests.
- Define peer and session lifecycle handling for QUIC, including stream and
  datagram semantics, rate limiting, and backpressure.
- Integrate QUIC with block-sync, fast-sync, and overlay communication paths.
- Expand public documentation and examples alongside stabilized APIs and CLI
  workflows.

## Later Backlog

These items remain intentionally postponed:

- Toncenter-compatible HTTP API client.
- WASM and no-std feasibility audits.
- Additional wallet contract builders.
- Additional jetton and NFT convenience packages.
- Storage daemon protocol support.
- Validator engine control API support.
