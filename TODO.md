# TODO

This file follows the `todo-md/todo-md` format and is the detailed tracker for
work that should not be hidden in prose. Active work is currently prioritized
around idiomatic high-level LiteClient, LiteBalancer, contract, wallet,
mnemonic, and ABI capabilities, plus post-Phase-1 protocol hardening.
`ROADMAP.md` explains the phase intent at a higher level; `dev-docs/` records
protocol facts and source evidence. Completed work moves to `# DONE`;
postponed work moves to `# BACKLOG`.

## High-Level SDK Capability Coverage

- [ ] Track LiteClient workflow coverage #liteclient #network #contracts #tests #docs
  - [-] Define expected success and failure behavior for connect, masterchain info, block lookup, account state, run method, send message, and raw query workflows #liteclient #network #tests #docs
    - [x] Document contract-facing account state, run method, send message, and transaction workflow behavior for LiteClient providers #liteclient #contracts #docs
    - [-] Complete connect, block lookup, and raw query acceptance coverage #liteclient #network #tests #docs
      - [x] Add offline acceptance tests for block lookup request construction and raw query byte preservation #liteclient #network #tests
      - [ ] Add connect-path acceptance coverage without live credentials #liteclient #network #tests
  - [ ] Map typed LiteClient helpers to current local LiteAPI schema coverage and accepted protocol evidence #liteclient #tl #docs
  - [-] Add fixture-backed tests for typed LiteClient payload decoding and error behavior #liteclient #tvm #tlb #tests
    - [x] Add offline tests for typed success decoding, unexpected response rejection, lookup result decoding, and malformed BoC decode errors #liteclient #tvm #tlb #tests
    - [ ] Add checked upstream or live-captured fixtures for representative block/account/proof payloads #liteclient #tvm #tlb #tests
  - [ ] Add ignored live-network smoke tests for public config workflows #liteclient #network #tests
- [ ] Track LiteBalancer workflow coverage #balancer #liteclient #network #tests #docs
  - [-] Define expected success and failure behavior for peer selection, retry, archival routing, and raw query delegation #balancer #network #tests #docs
    - [x] Document contract-facing account state, get-method, external-message, and transaction delegation parity with LiteClient #balancer #contracts #docs
    - [-] Complete peer selection, retry, archival routing, and raw query acceptance coverage #balancer #network #tests #docs
      - [x] Add offline tests for typed-helper retry routing, archival probe filtering, non-retryable server errors, and local decode errors #balancer #network #tests
      - [ ] Add raw query delegation coverage if a public balancer raw-query helper is introduced #balancer #network #tests
  - [-] Add verification coverage for peer health state transitions, failover, and request routing decisions #balancer #network #tests
    - [x] Verify current failure handling removes failed peers, records dead state, and clears in-flight counters #balancer #network #tests
    - [ ] Add reconnect and timeout state-machine coverage after those behaviors are implemented #balancer #network #tests
  - [-] Keep LiteBalancer typed helper coverage synchronized with LiteClient helper coverage #balancer #liteclient #docs
    - [x] Add offline parity checks for representative typed helper routing through peer selection, retry, and local decode error paths #balancer #liteclient #tests
    - [ ] Keep adding representative parity checks when LiteClient typed helper coverage expands #balancer #liteclient #tests
- [-] Maintain RPS limiting for rented liteserver quotas as a first-class SDK capability #balancer #liteclient #network #perf #tests
  - [x] Add API-level configuration for global and per-peer RPS caps #balancer #liteclient
  - [x] Enforce limiter in LiteClient and LiteBalancer request paths with clear error/throttle semantics #balancer #liteclient #network
  - [x] Add CLI options for rate limits where network calls are exposed #cli #network
  - [x] Document tonconsole-style quota usage patterns and safe defaults #docs #network
  - [x] Add deterministic tests for RPS limiter behavior: burst handling, steady-state throttle, per-peer quotas, and backoff timing #tests #balancer #network
  - [ ] Validate limiter behavior against live tonconsole-style rented liteserver credentials #network #tests
- [ ] Track contract workflow capabilities #contracts #liteclient #tvm #tests #docs
  - [x] Define get-method argument encoding, method id/name handling, TVM stack return decoding, and error semantics for contract wrappers #contracts #tvm #docs
  - [x] Add public get-method stack conversion traits and typed `Contract::*_as` helpers with arity, type, range, option, tuple, address, and cell tests #contracts #tvm #tests #docs
  - [-] Track state-init address derivation, data/code loading, deployment message construction, and balance access #contracts #wallet #tvm #docs
    - [x] Add state-init address derivation, active account balance/code/data helpers, and raw external message BoC submission #contracts #tvm #docs
    - [x] Add code-plus-data contract blueprint trait, derive macro, and address/bind tests #contracts #tvm #tests #features
    - [ ] Add wallet signing and full deployment message construction #contracts #wallet #tvm #docs
  - [-] Add fixture-backed tests for representative contract get-method and external-message workflows #contracts #tvm #tests
    - [x] Add mock-provider unit tests for contract get-method, state, transaction, state-init address, and external BoC routing semantics #contracts #tvm #tests
    - [ ] Add checked protocol fixtures or ignored live-network evidence for representative contract workflows #contracts #tvm #tests
- [-] Track wallet and mnemonic capabilities #wallet #contracts #crypto #tests #docs
  - [x] Define mnemonic generation, import, validation, and derivation behavior #wallet #crypto #docs
  - [-] Track wallet init/deploy, transfer/message signing, seqno, timeout, and state handling #wallet #contracts #crypto #docs
    - [x] Add Wallet V5R1 storage data, wallet-id packing, signed external body construction, external message BoC construction, and address derivation #wallet #contracts #crypto #tests
    - [x] Add Wallet V4R2 storage data, default wallet-id handling, signed simple-send body construction, external message BoC construction, and address derivation #wallet #contracts #crypto #tests
    - [x] Add CLI wallet generate, address, seqno, prepare-transfer, and send MVP without local secret storage #wallet #cli #network #crypto
    - [x] Add get-method helpers for V5R1 seqno, wallet id, public key, signature-auth status, and raw extension payload lookup #wallet #contracts #tvm #docs
    - [x] Add decoded Wallet V5R1 extension dictionary lookup and management policy #wallet #contracts #tvm #docs
    - [x] Add mock-provider coverage for wallet send routing through `send_external_message_boc` #wallet #cli #contracts #tests
    - [x] Promote live V5R1/V4R2 send/deploy helpers after deterministic fixtures and provider behavior are accepted #wallet #contracts #network #tests
  - [-] Add deterministic wallet and mnemonic fixtures before live send workflows are enabled #wallet #crypto #tests
    - [x] Add offline Wallet V5R1 tests for wallet-id vectors, data roundtrip, empty extensions, signed body construction, action-count limits, signature verification, and external message BoC decoding #wallet #crypto #tests
    - [x] Add offline Wallet V4R2 tests for data roundtrip, empty plugins, signed body construction, action-count limits, signature verification, and external message BoC decoding #wallet #crypto #tests
    - [x] Add mnemonic generation/import/derivation fixtures #wallet #crypto #tests
    - [x] Add deterministic Criterion benchmarks for mnemonic derivation, cached wallet code, address derivation, and signed transfer BoC construction #wallet #crypto #perf #tests
    - [x] Reconcile embedded `@ton/ton` Wallet V4R2/V5R1 code BoC hashes with current TON wallet-history documentation hashes #wallet #crypto #docs #tests
    - [x] Add upstream Wallet V5R1 and V4R2 state-init/address fixtures for mainnet and testnet defaults #wallet #crypto #tests
- [ ] Track ABI and typed wrapper capabilities #abi #contracts #wallet #tests #docs
  - [-] Map each ABI and wrapper gap to subsystem tags and acceptance tests #abi #contracts #docs #tests
    - [x] Add direct address-bound `Contract<'a, P>` typed-client delegation test #contracts #docs #tests
    - [x] Add derive-backed contract blueprint pattern for typed data and fixed code BoCs #contracts #docs #tests #features
    - [x] Add reusable typed TVM stack conversion traits for wrapper get-method arguments and results #contracts #tvm #tests #docs
    - [ ] Map ABI parser/encoder gaps to typed wrapper acceptance tests #abi #contracts #docs #tests
  - [ ] Keep capability tracker entries synchronized whenever implementation work completes or defers a gap #docs #tests

## TEP Metadata And Typed Wrappers

- [x] Implement common TEP metadata parsing #metadata #contracts #tvm #tests #docs
  - [x] Decode TEP-64 snake and chunked content cells with canonical bounds checks #metadata #tvm #tests
  - [x] Decode on-chain and off-chain URI content forms while preserving unsupported raw cells #metadata #tvm #tests
  - [x] Add raw-preserving fallback types for unknown keys, malformed values, and future TEP extensions #metadata #contracts
  - [x] Document metadata wire formats, known limits, and unsupported content forms in `dev-docs/` #metadata #docs
- [x] Add jetton metadata support for TEP-74 wrappers #metadata #jettons #contracts #tests
  - [x] Decode typed jetton master get-method output needed for metadata lookup #jettons #contracts #tvm
  - [x] Map TEP-64 jetton metadata keys into typed Rust fields with raw unknown-key preservation #metadata #jettons
  - [x] Add fixture-backed jetton metadata cells for on-chain and off-chain content #metadata #jettons #tests
- [x] Add NFT metadata support for TEP-62 wrappers #metadata #nft #contracts #tests
  - [x] Decode typed NFT collection and item get-method outputs needed for metadata lookup #nft #contracts #tvm
  - [x] Map TEP-64 NFT collection and item metadata keys into typed Rust fields with raw unknown-key preservation #metadata #nft
  - [x] Add fixture-backed NFT metadata cells for collection, item, and individual-content merge behavior #metadata #nft #tests

## ABI (Tongo-Level)

- [-] Implement full ABI data model coverage #abi #contracts #tvm
  - [x] Define ABI Rust types for contracts, methods, events, tuples, arrays, optional fields, and dictionaries #abi
  - [x] Define ABI scalar mappings for TON/TVM-relevant integer, bytes, address, bool, and cell-like values #abi #tvm
  - [x] Document ABI data-model invariants, numeric limits, and current non-goals in `dev-docs/` #abi #docs
- [ ] Implement ABI encoding and decoding engine #abi #contracts #tvm #tests
  - [-] Encode ABI inputs into TVM stack and message-body representations #abi #tvm
    - [x] Add value-level scalar TVM stack encode/decode helpers for integers, booleans, bytes, strings, addresses, cells, slices, tuples, arrays, and optionals #abi #tvm #tests
    - [x] Add ABI message-body encoding policy and implementation #abi #tvm #contracts #tests
    - [x] Add local fixed integer-key ABI map/dictionary stack and message-body codecs #abi #tvm #tests
  - [-] Decode get-method outputs and external message payload components from ABI definitions #abi #tvm
    - [x] Add ABI get-method input encoding and output decoding helpers over TVM stack entries #abi #tvm #tests
    - [ ] Add ABI event/external payload component helpers beyond full message-body decode #abi #tvm #tests
  - [-] Add edge-case coverage for tuples, nested arrays, optional values, and dictionary-like payloads #abi #tests
    - [x] Cover nested tuple, array, optional, and JSON loader cases for stack and schema paths #abi #tests
    - [x] Define and test local dictionary-like payload policy before enabling map codecs #abi #tests
- [-] Implement JSON ABI parser and loader #abi #contracts #tests
  - [x] Parse and validate ABI JSON schema with precise diagnostics #abi #tests
  - [x] Support loading ABI definitions for contract wrappers and CLI workflows #abi #contracts #cli
  - [-] Add schema validation tests for malformed or ambiguous ABI documents #abi #tests
    - [x] Add malformed missing-field and invalid integer-width coverage #abi #tests
    - [x] Add ambiguous selector and unsupported compatibility-shape coverage #abi #tests
- [ ] Integrate ABI with contract workflows #abi #contracts #liteclient #cli #tests
  - [x] Add ABI-driven get-method argument encoding for contract wrappers #abi #contracts
  - [x] Add ABI-driven external message body construction #abi #contracts
  - [x] Add ABI-driven CLI input/output paths where contract commands expose typed data #abi #cli #contracts
- [-] Add golden fixtures and cross-reference validation cases #abi #tests #docs
  - [x] Add checked synthetic fixture-backed encode/decode vectors for representative ABI JSON, get-method stack, and message-body paths #abi #tests
  - [x] Add structured ABI evidence metadata and opt-in templates for wallet `seqno` and TEP-74 `get_wallet_address` captures #abi #tests #docs
  - [ ] Cross-check ABI fixtures against upstream or live-captured contract evidence #abi #tests
  - [ ] Cross-check behavior against tongo-compatible expectations and TON protocol definitions #abi #tests #docs
  - [x] Define and document local ABI map/dictionary stack and message codec policy before enabling map conversion #abi #tvm #docs
  - [x] Document unsupported ABI map key families and upstream-validation follow-up tasks #abi #docs

## Subsequent Phases (Post-ABI)

- [ ] Design the pure Rust emulator and local LiteServer phase #emulator #liteserver #tvm #liteclient #network #tests #docs
  - [ ] Write `dev-docs/` design notes for emulator scope, trust assumptions, upstream TON source evidence, and unsupported execution paths #emulator #tvm #docs
  - [ ] Define emulator execution scope for account state, config, time/logical-time context, get-method execution, inbound message execution, transaction results, and action results #emulator #tvm #tests
  - [ ] Define the fixture strategy for comparing supported emulator paths against upstream TON or recorded live behavior #emulator #tvm #tests #docs
  - [ ] Define local LiteServer protocol and API scope for fixture-backed account, block, config, get-method, and send-message workflows #liteserver #liteclient #network #tests #docs
  - [ ] Plan feature gates, examples, CLI integration, and dev-test workflows for offline contract, wallet, jetton, LiteClient, and LiteBalancer tests #emulator #liteserver #liteclient #network #tests #docs
- [ ] Complete blockchain TL-B coverage from upstream `block.tlb` #tvm #tlb #docs #tests
  - [x] Add checked local `src/tlb/schemas/block.tlb` snapshot for currently implemented constructor families #tvm #tlb
  - [x] Add deterministic constructor summary checks for typed and raw-preserving block/config/proof families #tvm #tlb #tests
  - [ ] Sync the complete upstream `crypto/block/block.tlb` file with source commit/date and full constructor drift tests #tvm #tlb #docs #tests
  - [ ] Replace raw-preserving block, shard-state, config, and proof wrappers with generated or handwritten typed models where stable #tvm #tlb #proofs
  - [ ] Add fixture-backed block, shard-state, config-param, Merkle proof, and Merkle update roundtrips from upstream TON or live captures #tvm #tlb #tests #proofs
  - [ ] Expand `tlb-derive` with parameterized TL-B types, implicit or CRC tags if needed, ambiguous-prefix checks, and negative compile tests #tvm #tlb #tests #features
- [ ] Expand captured upstream/live fixture evidence beyond Phase 1 synthetic vectors #tvm #boc #tlb #tests
  - [ ] Add public liteserver account-state captures with endpoint, date, schema revision, and root hashes #tvm #liteclient #tests
  - [ ] Add upstream TON or pytoniq-derived proof BoCs for account, block, shard-state, config, Merkle proof, and Merkle update workflows #tvm #proofs #tests
  - [ ] Cross-check checked-in message and transaction fixtures against independent upstream or pytoniq serialization #tvm #tlb #tests

## Documentation

- [ ] Expand `dev-docs` into a complete internal TON reference #docs
  - [ ] Add exact source links and schema revision notes for every protocol document #docs
    - [ ] Record upstream TON commit or schema date used for each sync #docs #tl
    - [ ] Record docs.ton.org pages used for each conceptual section #docs
  - [ ] Add diagrams for request wrapping, ADNL handshake, BoC serialization, and balancer failover #docs
    - [ ] Keep diagrams as text or Mermaid so they remain reviewable #docs
  - [ ] Add examples for every public high-level API #docs
    - [x] Add LiteClient connect and `get_masterchain_info` example #docs #network
    - [x] Add smart-contract get-method example with typed stack values #docs #contracts
    - [x] Add LiteBalancer multi-peer example #docs #network
- [ ] Build complete public documentation for every public API feature #docs
  - [ ] Document the crate-level architecture and feature-gated module map #docs #features
    - [ ] Explain which APIs are available with default features #docs #features
    - [ ] Explain which APIs require `tl`, `tvm`, `adnl`, `adnl-tcp`, `liteclient`, `network-config`, or `cli` #docs #features
    - [ ] Keep feature documentation synchronized with `Cargo.toml` and CI checks #docs #features #tests
  - [ ] Add rustdoc examples for public modules, types, traits, and constructors #docs #tests
    - [ ] Cover ADNL transport setup and low-level message exchange #docs #network
    - [ ] Cover LiteClient connection, typed requests, raw requests, and timeout configuration #docs #liteclient
    - [ ] Cover LiteBalancer peer configuration, failover behavior, and request routing #docs #balancer
    - [ ] Cover TVM cells, builders, slices, BoC parsing, stack values, and address parsing #docs #tvm
    - [ ] Cover TL serialization, deserialization, boxed types, flags, vectors, and raw bytes #docs #tl
    - [ ] Cover network config loading and liteserver extraction #docs #network
    - [ ] Cover smart contract wrappers, get-method calls, and return decoding when the contract API lands #docs #contracts
    - [ ] Cover proof verification APIs when they land #docs #proofs
    - [ ] Cover DHT, overlay, and mempool APIs when they land #docs #dht #overlay #mempool
  - [ ] Add public API guides in `docs/` #docs
    - [x] Add `docs/getting-started.md` with minimal dependency and feature setup #docs #features
    - [x] Add `docs/liteclient.md` with typed and raw LiteAPI workflows #docs #liteclient
    - [x] Add `docs/balancer.md` with multi-peer failover workflows #docs #balancer
    - [x] Add `docs/tvm.md` with cells, BoC, stack, and address examples #docs #tvm
    - [x] Add `docs/tl.md` with schema, constructor id, and serialization notes #docs #tl
    - [x] Add `docs/networking.md` with ADNL, DHT, overlay, and transport notes #docs #network
    - [x] Add `docs/cli.md` with CLI usage, output formats, exit codes, and shell scripting patterns #docs #cli
    - [x] Add `docs/testing.md` with fixture, live-network, and ignored-test instructions #docs #tests
  - [ ] Keep docs testable and version-aware #docs #tests
    - [ ] Enable doctests for examples that do not require live network access #docs #tests
    - [ ] Mark live-network examples with explicit ignored-test instructions #docs #tests
    - [ ] Add a docs checklist to release preparation #docs
- [ ] Build an `examples/` suite covering every public API feature #docs #examples
  - [ ] Add minimal examples that compile under the narrowest required feature set #examples #features #tests
    - [x] `examples/liteclient_masterchain_info.rs` #examples #liteclient
    - [x] `examples/liteclient_raw_query.rs` #examples #liteclient #tl
    - [x] `examples/litebalancer_failover.rs` #examples #balancer
    - [x] `examples/network_config.rs` #examples #network
    - [x] `examples/adnl_ping.rs` or equivalent loopback-safe ADNL example #examples #network
    - [x] `examples/tvm_cell_builder.rs` #examples #tvm
    - [x] `examples/tvm_boc_roundtrip.rs` #examples #tvm
    - [x] `examples/tvm_stack_run_method.rs` #examples #contracts #tvm
    - [ ] `examples/proof_verify_account_state.rs` after proof APIs land #examples #proofs
    - [ ] `examples/mempool_stream.rs` after mempool APIs land #examples #mempool
  - [ ] Add example verification to CI #examples #tests
    - [ ] Compile examples with default features #examples #tests
    - [ ] Compile examples with all features #examples #tests
    - [ ] Compile feature-specific examples with explicit `--features` lists #examples #features #tests
  - [ ] Keep examples shell-friendly where applicable #examples #cli
    - [ ] Print machine-readable JSON for network examples when possible #examples #cli
    - [x] Avoid hidden environment assumptions in examples #examples #tests
    - [x] Document required public config or liteserver input for live examples #examples #docs
    - [ ] Add a stable documented default testnet contract for get-method examples #examples #contracts #tests
- [ ] Keep `TODO.md` detailed and todo-md compliant #docs
  - [ ] Move completed tasks to `# DONE` instead of deleting them #docs
  - [ ] Tag all tasks with subsystem tags #docs
  - [ ] Keep subtasks concrete enough to implement without rediscovery #docs

## Cargo Features And Dependency Shape

- [ ] Finalize feature matrix #features
  - [ ] Define `std`, `tl`, `tvm`, `adnl`, `adnl-tcp`, `liteclient`, `network-config`, and `cli` public expectations #features
    - [ ] Document which modules compile under each feature combination #features #docs
    - [ ] Add CI commands for default, no-default, and all-features builds #features #tests
  - [ ] Move optional dependencies behind the narrowest possible features #features
    - [ ] Audit `anyhow`, `chrono`, `num-bigint`, `async-trait`, and `bytes` usage #features #perf
    - [ ] Remove unused dependencies after the API stabilizes #features
  - [ ] Add feature-gated doctests where module availability changes #features #tests

## TL Schema And Code Generation

- [ ] Build a checked TL schema workflow #tl
  - [ ] Add a local tool that parses `src/tl/schemas/lite_api.tl` and computes constructor ids #tl
    - [ ] Compare computed ids with handwritten `#[tl(id = ...)]` values #tl #tests
    - [ ] Fail tests when upstream schema and Rust types drift #tl #tests
  - [ ] Decide whether generated Rust code replaces or validates handwritten `tl-proto` types #tl
    - [ ] Prototype generation for simple constructors #tl
    - [ ] Prototype generation for boxed enums and flags #tl
    - [ ] Keep generated output deterministic and formatted #tl
  - [ ] Sync local `lite_api.tl` fully with upstream TON #tl
    - [x] Add nonfinal candidate request types #tl #mempool
    - [x] Add pending shard block request types after schema sync from upstream TON (constructors absent in current local `src/tl/schemas/lite_api.tl`) #tl #mempool
    - [ ] Add missing debug verbosity type if needed #tl
    - [x] Verify `SignatureSet::ordinary` and `SignatureSet::simplex` roundtrips #tl #tests
  - [x] Add TL roundtrip tests for every request and response type #tl #tests
    - [x] Cover vectors, bytes padding, flags, optional fields, and boxed enums #tl #tests
    - [x] Add golden binary fixtures for high-risk constructors #tl #tests

## Native ADNL TCP

- [ ] Harden ADNL TCP transport #network #adnl
  - [ ] Add full loopback client/server handshake integration test #network #tests
    - [ ] Verify server decrypts client handshake and returns encrypted empty proof packet #network #tests
    - [ ] Verify client rejects invalid server proof or EOF #network #tests
  - [-] Add codec tests for boundary sizes #network #tests
    - [x] Test 64-byte minimum encrypted frame #network #tests
    - [ ] Test maximum accepted payload and too-large payload rejection #network #tests
    - [x] Test multiple frames in one buffer #network #tests
  - [x] Document and verify AES-CTR key and nonce directionality #network #crypto
    - [x] Ensure client rx/tx maps to server tx/rx exactly #network #crypto #tests
  - [ ] Add connection timeout and graceful close APIs #network
    - [ ] Expose configurable TCP connect timeout #network
    - [ ] Expose request timeout at LiteClient layer #network #liteclient
  - [ ] Replace lossy logging of TL bytes with structured trace helpers #network #tl

## LiteClient And LiteAPI

- [ ] Complete the typed LiteClient surface #liteclient #tl
  - [x] Add typed methods for every current LiteAPI function present in current local schema snapshot (`src/tl/schemas/lite_api.tl`) #liteclient
    - [x] `lookupBlockWithProof` #liteclient #tl
    - [x] `listBlockTransactionsExt` #liteclient #tl
    - [x] `getLibrariesWithProof` #liteclient #tl
    - [x] `getShardBlockProof` #liteclient #tl
    - [x] `getOutMsgQueueSizes` #liteclient #tl
    - [x] `getBlockOutMsgQueueSize` #liteclient #tl
    - [x] `getDispatchQueueInfo` #liteclient #tl #mempool
    - [x] `getDispatchQueueMessages` #liteclient #tl #mempool
    - [x] nonfinal validator group and candidate calls #liteclient #tl #mempool
  - [x] Make `query_raw` truly raw instead of requiring conversion back into known `Request` #liteclient #tl
    - [x] Add raw ADNL LiteAPI query path that accepts already serialized request bytes #liteclient #tl
    - [x] Return raw response bytes before typed decoding #liteclient #tl
  - [x] Add typed response helpers for raw BoC payloads #liteclient #tvm
    - [x] Decode `getAccountState.state` as a TL-B `Account` BoC root cell #liteclient #tvm #tlb
    - [x] Structurally inspect multi-root proof BoCs for root counts and hashes #liteclient #tvm #proofs
    - [x] Decode block and shard proofs into cells where possible #liteclient #tvm
    - [x] Add typed `LiteClient` helpers for block, account, transaction, shard, config, library, and get-method response payloads #liteclient #tvm #tlb
    - [x] Add matching typed `LiteBalancer` delegation helpers through peer selection, retry, and rate-limit logic #balancer #liteclient
    - [ ] Expand typed shard/config payloads beyond opaque cells after full `block.tlb` model generation lands #liteclient #tvm #tlb
  - [ ] Add ignored live-network tests #liteclient #tests
    - [ ] Fetch masterchain info from public config #liteclient #tests
    - [ ] Fetch version and time #liteclient #tests
    - [ ] Run a simple get-method against a known public contract #liteclient #contracts #tests

## LiteBalancer

- [ ] Replace prototype balancer with production behavior #balancer #network
  - [ ] Make peer state transitions explicit and tested #balancer #tests
    - [ ] Healthy to Suspect after timeout #balancer #tests
    - [ ] Suspect to Dead after repeated connection errors #balancer #tests
    - [ ] Dead to Recovering during reconnect #balancer #tests
    - [ ] Recovering to Healthy after successful probe #balancer #tests
  - [ ] Add reconnect manager #balancer #network
    - [ ] Store peer connection descriptors instead of only connected clients #balancer
    - [ ] Add exponential backoff with jitter #balancer #perf
    - [ ] Stop reconnect tasks cleanly on `close_all` #balancer
  - [ ] Improve scoring #balancer #perf
    - [ ] Use EWMA latency instead of arithmetic average #balancer #perf
    - [ ] Penalize stale masterchain seqno relative to best observed seqno #balancer
    - [ ] Penalize high in-flight request count #balancer #perf
  - [ ] Share request delegation logic instead of duplicating every LiteClient method #balancer
    - [ ] Add trait or macro only if it reduces duplication without hiding control flow #balancer
  - [ ] Add multi-peer send-message policy #balancer
    - [ ] Return success if any peer accepts the message #balancer
    - [ ] Preserve individual peer errors for diagnostics #balancer

## TVM Cells, BoC, Slice, Builder, Dictionary

- [ ] Make TVM primitives spec-accurate #tvm
  - [x] Audit ordinary cell hash computation against TON representation hash rules #tvm #tests
    - [x] Add golden cell hash fixtures #tvm #tests
    - [x] Add multi-level reference depth fixtures #tvm #tests
  - [-] Add exotic cell support #tvm
    - [x] Pruned branch #tvm
    - [x] Library reference #tvm
    - [x] Merkle proof #tvm
    - [x] Merkle update #tvm
    - [ ] Add multi-level hash and depth helper APIs for exotic proof verification #tvm #proofs
    - [ ] Add upstream or pytoniq-core golden fixtures for exotic cells and proof BoCs #tvm #boc #tests
  - [ ] Improve BoC serialization and deserialization #tvm
    - [ ] Support index table modes #tvm
    - [ ] Support cache bits where required #tvm
    - [ ] Validate CRC32C handling #tvm #tests
    - [-] Add malformed BoC tests #tvm #tests
  - [ ] Improve Builder and Slice APIs #tvm
    - [x] Add explicit big unsigned integer builder and slice APIs for values wider than 64 bits #tvm
    - [x] Add explicit signed big integer builder and slice APIs #tvm
    - [ ] Migrate protocol codecs to the explicit big integer APIs where key or value widths exceed 64 bits #tvm #tlb
    - [ ] Add zero-copy or low-copy bit operations where possible #tvm #perf
  - [ ] Replace placeholder dictionary implementation with TON HashmapE semantics #tvm
    - [x] Implement fixed-width `BitKey` storage and callback-based `HashmapE` APIs #tvm #tlb
    - [x] Implement canonical label encoding #tvm #tlb
    - [x] Implement fork nodes #tvm #tlb
    - [x] Implement augmentation-preserving `HashmapAug` and `HashmapAugE` APIs #tvm #tlb
    - [x] Add embedded synthetic offline fixtures for HashmapE and HashmapAugE canonical encodings #tvm #tests
    - [ ] Add official golden fixtures for HashmapE encodings #tvm #tests
    - [ ] Add official golden fixtures for HashmapAug encodings #tvm #tests
    - [ ] Add higher-level typed dictionary value codecs after core TL-B models exist #tvm #tlb
    - [ ] Implement proof-friendly traversal #tvm

## TVM Stack And Smart Contracts

- [-] Make TVM stack encoding compatible with LiteAPI `runSmcMethod` #contracts #tvm
  - [x] Encode root `VmStack.depth` as 24 bits for empty-stack live get-method calls #contracts #tvm #tests
  - [-] Verify non-empty stack BoC shape against TON node expectations #contracts #tests
    - [x] Add checked synthetic offline fixtures for non-empty scalar, deep chain, tuple/list, huge integer, cell/slice, and unsupported stack BoCs #contracts #tests
    - [x] Add ignored live smoke test for non-empty stack get-method calls #contracts #tests
    - [x] Add captured stack fixture schema and successful-live JSON printing workflow #contracts #tests
    - [ ] Compare with tonutils-go and tonlib behavior #contracts
    - [ ] Add golden fixtures from successful live calls #contracts #tests
  - [x] Support arbitrary precision integers #contracts #tvm
  - [x] Support tuple/list nesting beyond four direct entries #contracts #tvm
- [ ] Add high-level contract API #contracts
  - [x] Add contract blueprint helpers for deriving `StateInit`, address, and provider binding from fixed code and typed data #contracts #tvm
  - [ ] Add wallet helpers only after generic contract API is stable #contracts
  - [ ] Add jetton and NFT helpers behind optional features #contracts #features

## TON Blocks, Accounts, Transactions, And Messages

- [ ] Implement TL-B models for core blockchain data #tlb #tvm
  - [-] Message and CommonMsgInfo #tlb
    - [x] Implement hand-written `Message Any`, `CommonMsgInfo`, relaxed messages, and `StateInitWithLibs` #tlb #tvm
    - [x] Add `OutAction` and `action_send_msg` models #tlb #tvm
    - [x] Add `OutList` models for transaction action lists #tlb #tvm
    - [x] Add schema-exact `TrActionPhase` action metadata by action-list hash #tlb #tvm
    - [ ] Add golden BoC fixtures for real message encodings #tlb #tests
  - [x] Account and AccountState #tlb
  - [x] Full Transaction, transaction descriptions, and remaining phases #tlb
  - [x] Augmented shard/account-block transaction collection models #tlb #tvm
  - [-] Block header, value flow, extra, and shard hashes #tlb
    - [x] Add generated-backed `Block`, `ValueFlow`, `BlockExtra`, and `ShardState` wrappers that preserve raw child cells #tlb #tvm
    - [ ] Expand generated models for full `BlockInfo`, `ValueFlow`, `BlockExtra`, and shard-hash families #tlb #tvm
  - [-] Config parameters #tlb
    - [x] Add `ConfigParams` wrapper with `config_addr:bits256` and raw config dictionary reference #tlb #tvm
    - [x] Decode `ConfigParams.config` as `Hashmap 32 ^Cell` with raw-preserving wrappers for params 0, 1, 2, 15, 17, 18, 20, 21, 24, 25, 32, 34, and 36 #tlb #tvm
    - [ ] Generate typed config-param family models #tlb #tvm
- [-] Add proof verification primitives #proofs
  - [x] Add Merkle proof/update exotic wrappers and child virtual-hash checks #proofs #tvm
  - [ ] Verify account state proof from `getAccountState` #proofs #liteclient
  - [-] Extract `ShardAccount` and `last_trans_hash` from verified `ShardAccounts` proof paths #proofs #liteclient #tlb
    - [x] Add checked shard-account extraction API for proof-anchored roots with account hash, shard root, malformed BoC, and state/proof mismatch tests #proofs #liteclient #tests
    - [ ] Traverse full shard-state `ShardAccounts` dictionaries from live `getAccountState` proofs #proofs #liteclient #tlb
  - [ ] Verify shard inclusion proof #proofs #liteclient
  - [ ] Verify block proof links and signatures #proofs
  - [ ] Document trust assumptions for light client usage #proofs #docs

## DHT, Overlay, And Mempool

- [ ] Research and implement native ADNL UDP #network #adnl
  - [ ] Document packet format and channel negotiation #network #docs
  - [ ] Add UDP codec tests #network #tests
  - [ ] Add NAT and address list considerations #network
- [ ] Implement DHT discovery #dht #network
  - [ ] Add DHT TL types #dht #tl
  - [ ] Verify node signatures #dht #crypto
  - [ ] Resolve liteservers and overlay peers through DHT #dht
- [ ] Implement overlay protocol #overlay #network
  - [ ] Add overlay node and peer exchange types #overlay #tl
  - [ ] Add overlay query transport #overlay
  - [ ] Add broadcast handling where needed for mempool #overlay #mempool
- [ ] Build mempool scanning support #mempool
  - [ ] Study `yungwine/ton-mempool` behavior and map required overlay flows #mempool #docs
  - [ ] Identify public API for pending external messages #mempool
  - [ ] Add stream API for pending messages #mempool
  - [ ] Add backpressure and filtering #mempool #perf
  - [ ] Add tests with captured fixtures before live network tests #mempool #tests

## CLI And Shell Automation

- [ ] Turn the CLI into a complete scriptable surface for the public API #cli
  - [ ] Define CLI stability rules alongside the Rust public API #cli #docs
    - [ ] Document command naming, argument naming, output formats, and exit code conventions #cli #docs
    - [ ] Keep every stable CLI command covered by help text and `docs/cli.md` #cli #docs
    - [ ] Add deprecation rules for renamed commands and fields #cli #docs
  - [ ] Add machine-readable output controls #cli
    - [-] Support `--output json` for every command that returns structured data #cli
    - [-] Support `--output pretty-json` for interactive debugging #cli
    - [ ] Support `--output raw` or hex/base64 where commands return bytes #cli #tl
    - [-] Keep human output separate from stderr diagnostics #cli
    - [ ] Add stable error objects for JSON output #cli
  - [ ] Add configuration inputs suitable for shell scripts #cli
    - [ ] Accept liteserver config path, inline JSON, and environment variables #cli #network
    - [ ] Accept explicit peer address, public key, and timeout overrides #cli #network
    - [ ] Add `--mainnet`, `--testnet`, and custom config selection when network config support is stable #cli #network
    - [ ] Add `--timeout`, `--retries`, and `--failover` options for network commands #cli #balancer
  - [ ] Mirror LiteClient public API in CLI commands #cli #liteclient
    - [ ] `liteclient masterchain-info` #cli #liteclient
    - [ ] `liteclient time` and `liteclient version` #cli #liteclient
    - [ ] `liteclient raw-query` accepting hex, base64, or file input #cli #liteclient #tl
    - [ ] `liteclient block-header`, `lookup-block`, and block transaction listing commands #cli #liteclient
    - [ ] `liteclient send-message` with file, hex, or base64 BoC input #cli #liteclient
    - [ ] Add new LiteAPI commands whenever typed LiteClient methods are added #cli #liteclient
  - [ ] Mirror TVM and contract APIs in CLI commands #cli #tvm #contracts
    - [ ] `tvm boc decode`, `tvm boc encode`, and `tvm boc hash` #cli #tvm
    - [ ] `tvm cell inspect` for bits, refs, level, depth, and hash data #cli #tvm
    - [ ] `address parse` and `address format` #cli #tvm
    - [ ] `contract state` for account state loading #cli #contracts
    - [x] `contract run-get-method` with typed stack argument input #cli #contracts
    - [x] `contract run-abi-get-method` with ABI JSON argument input and output decoding #cli #contracts #abi
    - [x] `contract run-get-method` with JSON stack argument input for shell scripts #cli #contracts
  - [-] Add high-level default-balancer commands for common workflows #cli #balancer #liteclient
    - [x] Add `status`, `account`, `call`, `transactions`, `block`, and `config` commands #cli
    - [x] Add `--num-servers`, `--single`, and `--ls-index` backend selection #cli #network
    - [x] Add best-effort account-state CLI decode reporting #cli #tlb
    - [ ] Make high-level `transactions` fetch history after verified `ShardAccounts` last-transaction hash extraction lands #cli #proofs #liteclient
    - [x] Add JSON stack argument input for `call` #cli #contracts
    - [x] Expand stack argument kinds beyond int/null/cell/slice #cli #contracts #tvm
  - [ ] Add future protocol commands as APIs land #cli
    - [ ] Add proof verification commands when proof APIs land #cli #proofs
    - [ ] Add DHT lookup commands when DHT APIs land #cli #dht
    - [ ] Add overlay inspection commands when overlay APIs land #cli #overlay
    - [ ] Add mempool stream commands when mempool APIs land #cli #mempool
  - [ ] Add CLI regression tests #cli #tests
    - [ ] Test help output for every command and subcommand #cli #tests
    - [ ] Test JSON output shape with snapshots or stable fixtures #cli #tests
    - [ ] Test nonzero exit codes and stderr behavior #cli #tests
    - [ ] Test raw input modes for file, stdin, hex, and base64 #cli #tests

## Performance

- [-] Add benchmarks #perf
  - [x] ADNL encode/decode throughput #perf #network
  - [x] TL serialize/deserialize throughput #perf #tl
  - [x] Cell hash and BoC serialization throughput #perf #tvm
  - [x] Builder, slice, and TVM stack codec throughput #perf #tvm
  - [ ] Balancer request selection overhead #perf #balancer
- [ ] Reduce allocations in hot paths #perf
  - [ ] Audit ADNL codec buffer copies #perf #network
  - [ ] Audit TL bytes wrapping #perf #tl
  - [ ] Audit TVM bit-level writes and reads #perf #tvm
- [ ] Add optional instrumentation #perf
  - [ ] Keep metrics behind feature gate #features #perf
  - [ ] Avoid mandatory tracing dependencies in default build #features #perf

## Testing And CI

- [ ] Add CI matrix #tests
  - [ ] `cargo fmt --check` #tests
  - [ ] `cargo check --no-default-features` #tests #features
  - [ ] `cargo check` #tests
  - [ ] `cargo check --all-features` #tests #features
  - [ ] `cargo test` #tests
  - [ ] `cargo test --all-features` #tests #features
- [ ] Add fixture strategy #tests
  - [ ] Store binary fixtures with source notes #tests #docs
  - [ ] Keep live-network tests ignored by default #tests
  - [ ] Add deterministic random seeds where tests do not require cryptographic randomness #tests

# BACKLOG

- [ ] Add toncenter-compatible HTTP API client #http #features
- [ ] Add WASM support audit #wasm #features
- [ ] Add no-std feasibility audit #features
- [ ] Add wallet contract builders #contracts
- [ ] Add jetton and NFT convenience packages #contracts
- [ ] Add storage daemon protocol support #storage
- [ ] Add validator engine control API support #validator

# DONE

- [x] Establish protocol-compatible Address and ordinary BoC baseline #tvm #boc #address #tests #docs
  - [x] Document supported raw and user-friendly address formats in `dev-docs/tvm/addresses.md` #tvm #address #docs
  - [x] Add strict raw `workchain:hash` parsing and formatting helpers #tvm #address
  - [x] Support user-friendly base64 and base64url input with and without padding #tvm #address
  - [x] Preserve bounceable, non-bounceable, and test-only flags through parsing and explicit formatting helpers #tvm #address
  - [x] Add address validation tests for invalid tag, checksum, length, workchain, and hash input #tvm #address #tests
  - [x] Document ordinary-cell BoC support, index table handling, cache-bit rejection, and missing exotic-cell work in `dev-docs/tvm/boc.md` #tvm #boc #docs
  - [x] Decode ordinary generic BoC index tables when present #tvm #boc
  - [x] Reject cache-bit BoCs with a precise unsupported-feature error #tvm #boc
  - [x] Add BoC regression tests for indexed decode, malformed index table, CRC mismatch, invalid root/reference indexes, trailing bytes, and string roundtrips #tvm #boc #tests
  - [x] Add embedded TON Docs address fixtures and schema-derived BoC compatibility fixtures #tvm #boc #address #tests
  - [x] Add checked-in Phase 1 BoC fixture metadata for account/message/transaction compatibility #tvm #boc #tests
  - [x] Add checked-in synthetic ABI fixture metadata for JSON, get-method stack, message-body, and local map coverage #abi #tests
- [x] Complete remaining TVM primitive compatibility before TL-B macros #tvm #tlb
  - [x] Audit ordinary cell representation hash against TON golden fixtures #tvm #tests
  - [x] Add exotic cell support for pruned branch, library reference, Merkle proof, and Merkle update #tvm #boc
  - [x] Decide whether cache-bit BoCs can be decoded without semantic ambiguity and document the chosen behavior #tvm #boc #docs
  - [x] Add full TL-B schema macro design after Address and ordinary BoC behavior are stable #tvm #tlb #docs
- [x] Implement TL-B runtime trait foundation #tvm #tlb
  - [x] Add `TlbSerialize`, `TlbDeserialize`, `TlbScheme`, and `TlbError` with exact decode semantics #tvm #tlb
  - [x] Map TL-B codec errors to builder, slice, tag, reference, and non-canonical encoding failures #tvm #tlb #tests
  - [x] Add focused tests for tags, `Maybe`, `Either`, refs, `VarUInteger`, and trailing data #tvm #tlb #tests
- [x] Decide TL-B derive/proc-macro crate shape for Phase 1 #tvm #tlb #features
  - [x] Add optional feature-gated `tonutils-tlb-derive` proc-macro crate for custom TL-B structs #tvm #tlb #features
  - [x] Keep macro support optional and avoid adding compile cost to low-level TVM users #tvm #tlb #features
  - [x] Define schema-driven drift checks against upstream TON TL-B sources #tvm #tlb #tests
  - [x] Add public TL-B adapters for referenced values, raw cells, varuints, fixed-width fields, and typed `HashmapE` values #tvm #tlb
  - [x] Support binary and hex constructor tags in derive attributes #tvm #tlb #features
  - [x] Infer natural unsigned bit widths and support `u128` and `i128` fixed-width fields #tvm #tlb #features
  - [x] Add a standard jetton transfer derive example and public docs updates #docs #examples #tvm #tlb
- [x] Implement first core TL-B models from the documented design #tvm #tlb
  - [x] Start with hand-written `CommonMsgInfo`, `Message`, and `StateInit` codecs before deriving them #tvm #tlb
  - [x] Add focused unit tests for message address tags, values, state init references, common info variants, and `Message Any` placement #tvm #tlb #tests
  - [x] Record upstream schema source links for the message model family #tvm #tlb #docs
  - [x] Add `MsgAddress`, `CommonMsgInfoRelaxed`, `MessageRelaxed`, `SimpleLib`, and `StateInitWithLibs` in a follow-up message-model slice #tvm #tlb
  - [x] Add focused unit tests for relaxed addresses, relaxed message info variants, `MessageRelaxed Any`, and `StateInitWithLibs` libraries #tvm #tlb #tests
  - [x] Add hand-written `OutAction` and `LibRef` codecs for send-message, set-code, reserve-currency, and change-library actions #tvm #tlb
  - [x] Add focused unit tests for `OutAction` variants, referenced relaxed messages, library refs, and invalid action encodings #tvm #tlb #tests
  - [x] Add `OutList` linked-list models for transaction action phases #tvm #tlb
  - [x] Add schema-exact `TrActionPhase` metadata with `action_list_hash` and `StorageUsed` #tvm #tlb
  - [x] Add full transaction descriptions that reference `Maybe ^TrActionPhase` #tvm #tlb
  - [x] Add full top-level `Transaction`, `Account`, `HASH_UPDATE Account`, and transaction message dictionary models #tvm #tlb
  - [x] Add embedded synthetic offline BoC fixtures for implemented message, account, transaction, shard-account, and augmented dictionary models #tvm #tlb #boc #tests
  - [x] Add deterministic upstream-derived TL-B schema slice and checked generated summary for Phase 1 block/config/proof wrappers #tvm #tlb #tests
  - [x] Add generated-backed `Block`, `ShardState`, `ConfigParams`, Merkle proof, and Merkle update wrapper models for offline BoC inspection #tvm #tlb #proofs
  - [x] Add LiteClient BoC decode helpers that preserve raw bytes and expose typed Phase 1 views #liteclient #tvm #tlb #boc
  - [x] Add offline CLI BoC/TL-B decode and schema-check commands #cli #tvm #tlb #boc
  - [x] Add offline TL-B schema/codegen, BoC decode, LiteClient account-state decode, and guarded proof verification examples #docs #examples #tvm #liteclient #proofs
  - [x] Add fixture-backed roundtrip tests that compare checked-in Phase 1 message/account/transaction cell hashes #tvm #tlb #tests
  - [x] Add fixture-backed transaction-description BoCs for ordinary, tick-tock, split, and merge constructors #tvm #tlb #tests
- [x] Smoke-test live account proof BoC structural diagnostics for multi-root proof payloads #cli #liteclient #proofs #tests
- [x] Create `AGENTS.md` #docs
- [x] Create initial `dev-docs` directory #docs
- [x] Restructure `dev-docs` into subsystem directories #docs
- [x] Add `dev-docs/README.md` table of contents #docs
- [x] Add architecture, TL, network, TVM, LiteClient, contracts, research, and testing documentation sections #docs
- [x] Add blockchain, crypto, API, and operations documentation sections #docs
- [x] Create todo-md compliant `TODO.md` #docs
- [x] Add Cargo feature gates for default network-first build #features
- [x] Remove stale broken example target from mandatory test resolution #tests
- [x] Add ADNL codec roundtrip, partial frame, and tamper tests #network #tests
- [x] Add ADNL codec multi-frame and too-large payload tests #network #tests
- [x] Add ADNL loopback handshake test #network #tests
- [x] Add local LiteAPI schema constructor id checker #tl #tests
- [x] Make LiteClient raw query path preserve unknown request and response bytes #liteclient #tl
- [x] Add basic TVM stack representation and roundtrip tests #tvm #contracts
- [x] Add arbitrary precision integer support to TVM stack representation #tvm #contracts
- [x] Support TVM stack entry chains beyond four direct entries in the internal codec #tvm #contracts
- [x] Fix Builder bit and reference accounting #tvm
- [x] Fix 64-bit signed integer store/load edge cases #tvm
- [x] Add scriptable CLI command groups for LiteClient and LiteBalancer #cli #liteclient #balancer
- [x] Add CLI JSON, pretty JSON, raw, hex, and base64 output modes #cli
- [x] Add CLI raw LiteAPI query input modes for hex, base64, file, and stdin #cli #tl
- [x] Add network config liteserver selection helpers for public API constructors #network #liteclient
- [x] Add LiteClient constructors from parsed network config and liteserver entries #liteclient #network
- [x] Add initial public `docs/` guides for getting started, LiteClient, CLI, and examples #docs
- [x] Add initial compiling examples for network config, LiteClient masterchain info, and raw LiteAPI query #examples #liteclient
- [x] Add `docs/contracts.md` with smart contract usage patterns #docs #contracts
- [x] Add `examples/contract_get_method.rs` and `examples/contract_get_state.rs` #examples #contracts
- [x] Add `Contract` wrapper over `LiteClient` and `LiteBalancer` #contracts
  - [x] Fetch account state #contracts
  - [x] Run get-method by numeric id #contracts
  - [x] Run get-method by method name #contracts
  - [x] Decode common return shapes #contracts
- [x] Decode `RunMethodResult.result` into typed stack entries when supported #contracts #tvm
- [x] Preserve unsupported get-method result bytes losslessly #contracts #tvm
- [x] Add `contract state` and empty-stack `contract run-get-method` CLI commands #cli #contracts
