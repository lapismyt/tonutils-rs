# Internal Reference

This directory contains maintainer-facing notes for protocol ownership, wire
formats, error boundaries, async exceptions, and deterministic verification.
User-facing usage guidance lives in the root `README.md` and rustdoc. Do not
recreate the removed user-guide site or add parallel documentation trees.

## TL Ownership Map

TL snapshots and generated metadata stay centralized, while handwritten wire
types live with the protocol module that serializes them.

| Location | Owns | Does not own |
| --- | --- | --- |
| `crate::tl` | Shared TL primitives such as `TlInt256`, `TlString`, and `True`; serialization helpers; checked-in `.tl` snapshots; generated constructor metadata; cross-schema and schema-audit tests | Protocol-specific handwritten request, response, ADNL, or LiteAPI types |
| `crate::adnl::tl` | ADNL messages and packet contents, ADNL network addresses, DHT types, QUIC types, and ADNL TL public keys (`AdnlTlPublicKey`, `AdnlTlPublicKeyOwned`, `AdnlTlPublicKeyRef`) | LiteAPI wire types |
| `crate::liteclient::tl` | All `liteServer.*` requests and responses, LiteAPI identifiers and signatures, block links, nonfinal types, `LiteServerError`, `LiteServerAccountState`, and `LiteTransactionId` | Shared TL primitives or ADNL transport messages |

The checked-in snapshots are:

- `crates/tonutils/src/tl/schemas/lite_api.tl`
- `crates/tonutils/src/tl/schemas/ton_api.tl`
- `crates/tonutils/src/tl/schemas/tonlib_api.tl`
- `crates/tonutils/src/tlb/schemas/block.tlb`

Their generated metadata lives under `crates/tonutils/src/tl/generated/` and
`crates/tonutils/src/tlb/generated/`. The schema generator owns the
`*ConstructorMetadata` names:
`LiteApiConstructorMetadata`, `TonApiConstructorMetadata`,
`TonlibApiConstructorMetadata`, and `BlockTlbConstructorMetadata`.

Canonical public names are intentionally unambiguous. ADNL uses
`AdnlNetAddress` and `AdnlTlPublicKey*`; LiteAPI uses `LiteBlockId`,
`LiteBlockIdExt`, `LiteAccountId`, `LiteTransactionId`,
`LiteTransactionId3`, `LiteSignature`, `LiteSignatureSet`, `LiteBlockLink`,
`LiteZeroStateIdExt`, `LiteLibraryEntry`, and the `LiteNonfinal*` types.
There are no compatibility aliases for the old names.

## Wire Invariants

- TL constructor identifiers are 32-bit little-endian values.
- The distinction between boxed objects and bare field types is part of the
  schema. For example, `AdnlIdShort` is bare inside `adnl.packetContents` and
  needs its constructor prefix only when serialized as a top-level object.
- LiteAPI common identifiers are encoded with their TON `int`, `long`, and
  256-bit field widths. Do not widen or normalize them at the wire boundary.
- TL-B decoding is exact at schema boundaries. Trailing bits or references are
  an error unless the schema explicitly owns them.
- Ordinary cells hold at most 1,023 data bits and four references. Builders and
  slices reject overflow rather than truncating it.
- Exotic cells and BoC roots retain their original bytes where the typed layer
  does not yet model them. Generic BoC decoding is not a proof.
- A representation hash is computed over the canonical cell representation,
  not over the serialized BoC envelope. Proof helpers compare the roots they
  actually decoded, and callers must opt into the relevant checks.

## Provider Adapters

The `provider` feature exposes capability traits rather than one monolithic
provider. Callers should require only the traits they use; the `Provider` marker
is a convenience bound for implementations that support every capability.

| Adapter | Default endpoint | Explicit block | Method ids | Notes |
| --- | --- | --- | --- | --- |
| `LiteClient` | Configured liteserver | Yes | Yes | Direct LiteAPI calls |
| `LiteBalancer` | Selected liteservers | Yes | Yes | Balanced LiteAPI calls |
| `TonApi` | `https://tonapi.io` (`testnet.tonapi.io`) | No | No | Bearer token |
| `ToncenterV2` | `https://toncenter.com/api/v2` | Yes via `seqno` | Yes | `X-API-Key` |
| `ToncenterV3` | `https://toncenter.com/api/v3` | No | Yes | `X-API-Key` |

`None` as a block argument always means the latest block. HTTP adapters convert
stack values through `provider::stack`, and unsupported stack entry kinds return
`ProviderError::UnsupportedStackEntry` instead of being silently dropped.

### Trust Boundary

HTTP adapters trust the remote service. They do not verify Merkle proofs, block
signatures, or proof roots. Lite adapters talk directly to liteservers but also
return the response as received; use the typed `liteclient::boc` proof helpers
when verification is required. Explicit-block requests are rejected by adapters
that cannot address them (`ProviderError::UnsupportedBlock`).

## Typed Error Boundaries

- `tvm::TvmError` is the base error for cells, slices, builders, dictionaries,
  BoC, addresses, stacks, integer widths, hex, and base64. `tvm::TvmResult<T>`
  is used throughout the TVM layer.
- `tlb::TlbError::Tvm(TvmError)` preserves the low-level source while
  `TlbError` keeps schema-specific variants for tags, trailing data,
  non-canonical values, invalid references, and custom schema validation.
  `tlb::TlbResult<T>` is the TL-B result alias.
- Dictionary callbacks accept `TvmError`. When a TL-B codec crosses that
  boundary, `TvmError::Callback` carries the structured `TlbError` opaquely and
  `TlbError::from(TvmError)` recovers it by downcast, so schema variants are not
  flattened into strings.
- `wallet::WalletError::Tvm(TvmError)` and
  `wallet::WalletError::Tlb(TlbError)` keep construction failures typed.
- Provider capabilities return `provider::ProviderError`, which preserves HTTP,
  URL, API status, JSON decode, invalid-response, unsupported-capability,
  TVM-stack, LiteClient, and LiteBalancer sources.
- Contract helpers expose `ContractError::Provider(ProviderError)`,
  `ContractError::Decode(TvmError)`, `ContractBuildError::InvalidCodeBoc(TvmError)`,
  and `ContractBuildError::StateInitSerialization(TvmError)`. Stack conversion
  has its own typed error rather than being stringified.
- LiteClient BoC helpers return `liteclient::boc::LiteBocError` for TVM, TL-B,
  root-hash mismatch, account mismatch, non-standard account addresses, and
  account-state mismatch. `LiteError::Boc(LiteBocError)` and
  `LiteError::Decode(TvmError)` preserve those sources at the client boundary.
- The library must not use `anyhow` in production code or the normal dependency
  graph. `anyhow` remains available to tests and the CLI binary, where
  application-level context is useful.

Error messages and validation order are observable. When splitting a generic
failure into a typed variant, preserve the old context in the variant message
or its `#[source]` chain.

## Schema Workflow

`tonutils-schema-gen` is deterministic and does not fetch network data. It
parses the checked-in snapshots, validates constructor identities, rewrites
generated metadata, and produces
`docs/reference/schema-inventory.tsv`.

The inventory contains one tab-separated row per schema with these fields:
source path, schema kind, source SHA-256, constructor count, upstream revision,
generated-file SHA-256, and generated module path.

To validate without changing files:

```bash
cargo run -p tonutils-schema-gen -- check
```

To inspect the deterministic inventory:

```bash
cargo run -p tonutils-schema-gen -- --inventory
```

To rewrite generated metadata and the inventory after an intentional,
upstream-backed schema change:

```bash
cargo run -p tonutils-schema-gen -- --write
```

Commit the schema source, generated Rust modules, and inventory together.
Never hand-edit a generated file. Update the owning handwritten wire types and
their serialization vectors in the same change.

## Async I/O And Concurrency

Network and filesystem work stays on Tokio on every normal async path:

- `QuicServer::bind` and `QuicSession::connect` bind through
  `tokio::net::UdpSocket` before handing the socket to Quinn.
- The UDP demultiplexer heartbeat reads `/proc/net/udp` with
  `tokio::fs::read_to_string` on Linux.
- Capture records are formatted on the receive path but written by a dedicated
  bounded `adnl-capture-writer` thread. The queue never blocks the receiver,
  and the cap and JSONL format are preserved.
- Locks held across an `.await` are Tokio locks. Standard-library mutex and
  RwLock guards are limited to short critical sections that never cross an
  await point.
- ADNL peer and session maps use Tokio locks because their critical sections
  cross async receive and fan-out operations.

The following synchronous mechanisms are intentional exceptions, not missed
conversions:

- The process-wide `adnl-udp-inbound` thread owns a current-thread Tokio runtime
  for inbound ADNL work. This mirrors upstream's dedicated receive thread and
  prevents application task queues from delaying liveness-critical answers.
- The inbound executor may use its synchronous non-blocking shared-socket send
  path because it runs on that dedicated thread.
- The capture writer uses synchronous file I/O on its own worker thread so the
  async receive path performs no file I/O.
- `SocketAddr`, `IpAddr`, `Duration`, `Instant`, `Arc`, and atomics remain
  standard-library types. Their synchronous APIs are short, non-blocking, and
  do not own runtime I/O.

## Trust Boundary

Offline operations are deterministic and do not contact a node: cell and BoC
construction, TL/TL-B encoding and decoding, wallet address derivation,
signing, and external-message construction.

Live operations cross a trust boundary: ADNL transport, LiteAPI queries, and
message submission. The application
selects the network, liteserver, public keys, timeout and retry policy, and
proof-validation policy. A decoded response is not automatically a verified
response, and a successful submission is not inclusion or finality proof.
Proof helpers verify only the relations they document over the roots they
actually decoded.

## Verification

Run the complete local gate from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p tonutils-schema-gen -- check
RUSTDOCFLAGS="-D warnings" cargo doc -p tonutils --all-features --no-deps
```

Live network gates are ignored by default because they need public TON access.
Run them through `scripts/run-live-test.sh`, which saves the complete log and
fails unless the exact filter executes exactly one test and reports
`test <name> ... ok`:

```bash
scripts/run-live-test.sh live_probe mainnet_dht_find_node_returns_valid_nodes
scripts/run-live-test.sh lib liteclient::tests::live_workflows::live_get_masterchain_info_smoke
scripts/run-live-test.sh lib liteclient::tests::live_workflows::live_get_version_and_time_smoke
TON_GLOBAL_CONFIG_JSON="$TON_TESTNET_GLOBAL_CONFIG_JSON" \
  scripts/run-live-test.sh lib liteclient::tests::live_workflows::live_testnet_liteserver_smoke
```

`TON_GLOBAL_CONFIG_JSON` and `TON_TESTNET_GLOBAL_CONFIG_JSON` are required for
their respective network gates. The DHT gates derive seeds directly from
those configs. CI runs every live step with `if: ${{ !cancelled() }}` and
uploads the saved logs, so one failure does not hide evidence from the remaining
checks. Testnet DHT runs strictly from the scheduled/manual
`.github/workflows/live-testnet-dht.yml` workflow because the canonical testnet
seeds are currently unreachable; that workflow stays red until upstream
recovers.

- DHT evidence: config seed, remote peer, `dht.findNode`, elapsed time, raw and
  valid node counts. A nonempty all-valid answer is required.
- Liteserver evidence: typed `getMasterchainInfo`, `getVersion`, and `getTime`
  responses. The testnet smoke tries every configured index and reports the
  first one that answers `getMasterchainInfo`.

There is no network-unavailable bypass. Missing configuration, an unreachable
network, an empty response, or no public payload is a failure.

Check each standalone feature surface:

```bash
for feature in adnl tl tvm tlb network-config metadata liteclient contracts wallet crc serde; do
  cargo check -p tonutils --no-default-features --features "$feature"
done
```

Inspect the normal dependency graph when dependency features change:

```bash
cargo tree -p tonutils -e normal
```

The normal `tonutils` graph must not contain `anyhow`.
