# Roadmap

This roadmap describes the intended development phases for `tonutils-rs`. It is a
high-level planning document: `TODO.md` remains the detailed task tracker, and
`dev-docs/README.md` is the entry point for protocol and implementation notes.

## Direction

`tonutils-rs` is a pure Rust TON SDK inspired by `tonutils-go`. The crate should
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

Phase 1 has closed as the SDK foundation needed for TVM, BoC, TL, TL-B, and
contract ergonomics. The immediate priority moves to ergonomic LiteClient,
LiteBalancer, contract, wallet, and ABI capabilities:

1. Complete TVM types.
2. Complete BoC handling, including an `Address` type with raw,
   user-friendly, bounceable, non-bounceable, testnet, base64/base64url,
   hex/raw, external-address, and precise parse/format capabilities.
3. Use the supported TL-B schema parser/check-summary workflow for schema work.
4. Expand user-defined TL and TL-B schemas where higher-level APIs need them.
5. Add broader built-in TL and TL-B schemas beyond the Phase 1 surface.
6. Close important LiteClient and LiteBalancer methods for contract data,
   balance, transactions, code, and data.
7. Add custom smart-contract client support with idiomatic Rust wrappers.
8. Add built-in smart-contract wrappers for wallets and jettons.
9. Add an ABI module organized by protocol, version, and contract family.
10. Keep reusable get-method argument/result abstractions complete enough for
   wrapper work: the public conversion traits now cover common scalar, address,
   cell, option, and tuple cases, with fixture and live-network expansion still
   tracked in `TODO.md`.
11. Expand typed metadata parsing for Jettons, NFTs, and future TEP-compatible
   contract families while preserving unsupported raw content.

Hardening, productionization, and broad protocol expansion remain important but
are intentionally deferred until these foundation, contract, and ABI milestones
are complete.

## Phase 1: TVM, BoC, TL, And TL-B Foundation

Status: closed on 2026-05-09 as a compatibility foundation milestone.

Build the low-level primitives needed to decode, encode, and model TON data
without depending on third-party Rust TON SDK crates:

- Complete all required TVM value types and make `Cell`, `Slice`, `Builder`,
  stack values, dictionaries, and numeric codecs spec-accurate.
- Complete BoC serialization and deserialization, including common BoC variants,
  CRC handling, index/cache-bit behavior where required, exotic cells, golden
  fixtures, and string conversions.
- Bring `Address` to complete user-facing capability: raw, user-friendly,
  bounceable/non-bounceable, testnet, base64/base64url, hex/raw forms, external
  addresses, and precise parse/format validation.
- Add macro support for TL-B definitions so crate code can express cell-level
  schemas with checked serialization and deserialization.
- Add support for user-defined TL and TL-B schemas, while keeping checked schema
  maintenance and generated or derived code deterministic.
- Add built-in TL and TL-B schemas for core TON objects needed by the SDK,
  including messages, accounts, transactions, blocks, config objects, wallet
  payloads, and jetton payloads.

Exit criteria for Phase 1:

- BoC strings can be parsed into cells and serialized back across supported
  ordinary and required exotic cell cases.
- Core TL-B data models roundtrip through cells with golden fixtures.
- Custom TL/TL-B schemas can be defined by crate users through the supported
  macro/schema workflow.
- Address behavior is protocol-correct and covered by TON Docs fixtures.

Current Phase 1 fixture status:

- Address parsing and formatting are locked against embedded TON Docs vectors
  for raw, bounceable, non-bounceable, test-only, URL-safe, and standard
  base64 forms.
- Ordinary BoC and library-reference exotic BoC behavior are locked against
  small schema-derived embedded fixtures, including indexed decode and cache-bit
  rejection policy.
- TL-B runtime and macro direction is documented in `dev-docs/tvm/tlb.md`,
  including intended trait shape, constructor tag handling, references,
  `Maybe`, `Either`, `VarUInteger`, `HashmapE`, and error behavior.
- The minimal TL-B runtime trait foundation is implemented in `src/tlb/mod.rs`,
  including exact top-level decode, fixed tag helpers, `Maybe`, `Either`,
  referenced value helpers, canonical `VarUInteger` checks, and focused unit
  coverage.
- The first hand-written TL-B blockchain model slice is implemented in
  `src/tlb/message.rs`, covering `Anycast`, internal and external message
  addresses, `Grams`, `CurrencyCollection`, `TickTock`, current upstream
  `StateInit`, `CommonMsgInfo`, and `Message Any`.
- The next hand-written message slice is implemented in `src/tlb/message.rs`,
  covering `MsgAddress`, `CommonMsgInfoRelaxed`, `MessageRelaxed Any`,
  `SimpleLib`, and `StateInitWithLibs`.
- The closed `OutAction` family and `LibRef` are implemented in
  `src/tlb/message.rs`, including send-message, set-code, reserve-currency,
  and change-library actions.
- `OutList` is implemented in `src/tlb/message.rs` for transaction action
  linked lists, with the upstream 255-action limit enforced.
- Schema-exact `TrActionPhase` metadata is implemented in `src/tlb/message.rs`,
  including `AccStatusChange`, `StorageUsed`, optional fees/result argument,
  `uint16` counters, and `action_list_hash:bits256` without embedding `OutList`.
- Transaction-description models are implemented in `src/tlb/transaction.rs`,
  including storage, credit, skipped/VM compute, bounce, split/merge info, and
  all `TransactionDescr` constructors. `Maybe ^TrActionPhase` is represented as
  `Option<TrActionPhase>` with exact referenced child-cell encoding.
- Account and top-level transaction models are implemented in
  `src/tlb/transaction.rs`, including `StorageExtraInfo`, `StorageInfo`,
  `AccountState`, `AccountStorage`, `AccountStatus`, `Account`, `ShardAccount`,
  concrete `HASH_UPDATE Account`, and `transaction$0111` with exact referenced
  inbound/outbound message payloads. Split/merge install
  `prepare_transaction:^Transaction` fields now decode as boxed `Transaction`
  values.
- Augmented dictionary support is implemented in `src/tvm/dict.rs` with
  `HashmapAug` and `HashmapAugE`, preserving leaf, fork, and top-level
  augmentation values. The account-block slice in `src/tlb/transaction.rs`
  covers `DepthBalanceInfo`, `ShardAccounts`, `AccountBlock`, and
  `ShardAccountBlocks`.
- The currently implemented TL-B message, account, transaction, shard-account,
  and augmented dictionary model surface is locked by small embedded synthetic
  offline BoC fixtures with expected root hashes, exact decode checks, and
  canonical reserialization checks.
- Phase 1 now has a deterministic upstream-derived TL-B schema slice for
  block/config/proof families in `src/tlb/schemas/block_phase1.tlb`, a checked
  generated summary in `src/tlb/generated/block_phase1.rs`, and schema drift
  tests in `src/tlb/schema.rs`.
- Generated-backed Phase 1 wrappers cover `ShardIdent`, `ExtBlkRef`,
  `BlockIdExt`, `Block`, `ValueFlow`, `BlockExtra`, `ShardState`,
  `ConfigParams`, and exotic Merkle proof/update primitives while preserving
  raw child cells for deeper generated families that remain follow-up work.
- LiteClient BoC helpers preserve raw bytes and decoded root cells for semantic
  payloads, and structurally inspect multi-root account proof BoCs for root
  counts and representation hashes. They expose typed views for account `state`
  cells, block, config, shard-state, and single-root proof payloads. The CLI can
  decode BoCs, inspect known TL-B roots, and verify the schema snapshot offline.
- The CLI now has default-balancer high-level commands for status, account
  state, get-method calls, transactions, blocks, and config retrieval while
  retaining advanced `liteclient`, `balancer`, and raw compatibility commands.
- `fixtures/phase1/` now contains checked-in BoC metadata and bytes for
  message, relaxed-message, account, transaction, and all Phase 1
  transaction-description constructor families. Normal library tests decode
  these offline, compare root representation hashes, decode TL-B shape, and
  require canonical reserialization.
- Phase 1 TL-B macro support includes the `src/tlb/schema.rs` parser and
  deterministic checked-summary workflow plus the optional
  `tonutils-tlb-derive` workspace proc-macro crate for custom TL-B structs and
  enums.
- Full deep block/header/value-flow models, typed config-param families, and
  broader captured live/upstream proof fixture evidence remain follow-up work
  tracked in `TODO.md`.

## Phase 2: LiteClient, Contract Clients, Wrappers, Metadata, And ABI

Build ergonomic high-level SDK surfaces on top of the TVM, BoC, TL, and TL-B
foundation:

- Close important LiteClient and LiteBalancer methods for contract workflows:
  fetch contract state, balance, transactions, code, data, raw state, and
  get-method results with typed decoding where available.
- Add custom smart-contract client support. The minimum surface must support
  contract data serialization/deserialization, address computation from state
  init plus workchain, deployment by external message, balance lookup, and
  user-defined contract methods.
- Add built-in smart-contract wrappers. Initially include wallet wrappers for
  V4, V5, and Highload wallets, plus jetton wrappers based on a selected
  available contract variant.
- Add TEP metadata parsing before or alongside ABI work. Initial coverage must
  include a common raw-preserving metadata cell parser, TEP-64 on-chain and
  off-chain content handling, jetton metadata for TEP-74 wrappers, and NFT item
  and collection metadata for TEP-62 wrappers.
- Grow metadata parsing into a reusable contract metadata layer. It should cover
  Jetton, NFT item, NFT collection, and future TEP-compatible metadata formats;
  expose typed fields for common keys; keep unknown, malformed, or unsupported
  content raw-preserved; and stay usable independently of concrete wrapper
  implementations.
- Add get-method conversion traits similar in role to ton-rs `ToTVMStack` and
  `FromTVMStack`, but shaped idiomatically for this crate. They should support
  composing typed Rust arguments into TVM stack values, decoding stack results
  into typed Rust values, surfacing precise conversion errors, and reusing the
  same conversions from contract wrappers, ABI helpers, and CLI input/output.
- Add an `abi` module split by protocol and then by version or contract family.
  Initial ABI coverage must include wallets V4, V5, Highload, and jettons
  TEP-74 and TEP-89.
- Start wallet wrappers with Wallet V5R1 and V4R2. The first executable
  milestone is offline-safe: storage data cells, wallet-id packing, TON
  mnemonic derivation, signed external body construction, external message BoC
  construction, address derivation, and deterministic tests before live sending
  workflows are promoted.
- Keep a clear distinction between a concrete contract wrapper and an ABI
  description. As in tongo, contracts with different code must still work when
  they support the required methods and message shapes.
- Reuse the previously added TVM, BoC, TL, TL-B, and macro features for wrapper
  and ABI implementation instead of duplicating serialization logic.

Exit criteria for Phase 2:

- Users can implement their own contract clients with typed data, methods,
  state-init address derivation, deployment, and balance access.
- Built-in wallet and jetton wrappers cover the initial contract families,
  starting with Wallet V5R1.
- Jetton and NFT wrappers can decode supported TEP-64 metadata content while
  preserving unsupported raw content for follow-up parsing.
- Contract metadata parsing is available as a reusable raw-preserving layer for
  Jettons, NFTs, and later TEP-compatible contract families.
- Contract wrappers can express get-method arguments and results through typed
  TVM stack conversion traits instead of ad hoc stack assembly and decoding.
- ABI definitions can be used independently from concrete code hashes when the
  method and message interfaces are compatible.
- LiteClient and LiteBalancer expose the contract data retrieval methods needed
  by the wrappers and ABI layer.

## Phase 3: Hardening, Reliability, And Productionization

After the foundation, contract, and ABI milestones:

- Harden ADNL TCP behavior around boundaries, timeouts, graceful close, and
  structured diagnostics.
- Replace prototype balancer behavior with explicit peer states, reconnects,
  backoff, scoring, and clean shutdown.
- Stabilize CLI behavior, JSON error objects, and remaining machine-readable
  output contracts across supported commands.
- Make TVM cell, BoC, slice, builder, dictionary, and stack behavior fully
  spec-accurate with expanded golden fixtures.
- Add full proof verification models, including `ShardAccounts` path extraction
  for account proofs, and trust documentation for light client usage.

## Phase 4: Performance, Extended Protocols, And Ecosystem Coverage

After production hardening:

- Add benchmarks and allocation audits for ADNL, TL, TVM, BoC, and balancer
  hot paths.
- Implement ADNL UDP, DHT, overlay, and mempool scanning APIs with captured
  fixtures and later live-network tests.
- Treat ADNL UDP, DHT, and overlay as prerequisites for the pure Rust emulator
  and local LiteServer phase.
- Expand docs/examples coverage to match the finalized high-level APIs and
  CLI workflows.

## Phase 5: Pure Rust Emulator And Local LiteServer

After the TVM, TL-B, LiteAPI, proof, ADNL UDP, DHT, and overlay foundations are
mature, build local, embeddable infrastructure on top of this crate:

- Add a pure Rust TVM/account-state emulator for offline get-method execution
  and message execution. It must model account state, config parameters,
  time/logical-time context, inbound message execution, transaction results, and
  action results closely enough for deterministic contract tests.
- Add a pure Rust LiteServer-compatible local service for development fixtures,
  controlled integration environments, and SDK self-tests. It should handle
  LiteAPI requests over the existing TL and ADNL layers and serve deterministic
  fixture-backed account, block, config, get-method, and send-message surfaces.
- Support practical testing workflows: offline contract tests, wallet and
  jetton integration tests, reproducible CLI tests, and local network
  simulation.
- Keep the implementation autonomous and optional: no native `.so` emulator
  dependency, no third-party Rust TON SDK dependency, and feature-gated modules
  only.
- Continue using upstream TON behavior as the source of truth for supported
  execution paths, wire formats, failure modes, and fixture validation.

Exit criteria for Phase 5:

- Contract wrappers can run deterministic offline get-method and
  message-execution tests.
- The local LiteServer can serve enough LiteAPI to satisfy this crate's
  LiteClient and LiteBalancer integration tests.
- Emulator outputs are fixture-checked against upstream TON or recorded live
  behavior for supported paths.

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
