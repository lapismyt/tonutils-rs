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
| `crate::tl` | Shared TL primitives such as `TlInt256`, `TlString`, and `True`; serialization helpers; checked-in `.tl` snapshots; generated constructor metadata; cross-schema and schema-audit tests | Protocol-specific handwritten request, response, ADNL, LiteAPI, or overlay types |
| `crate::adnl::tl` | ADNL messages and packet contents, ADNL network addresses, DHT types, QUIC types, and ADNL TL public keys (`AdnlTlPublicKey`, `AdnlTlPublicKeyOwned`, `AdnlTlPublicKeyRef`) | LiteAPI or overlay wire types |
| `crate::liteclient::tl` | All `liteServer.*` requests and responses, LiteAPI identifiers and signatures, block links, nonfinal types, `LiteServerError`, `LiteServerAccountState`, and `LiteTransactionId` | Shared TL primitives or ADNL transport messages |
| `crate::overlay::tl` | Overlay and full-node types, capability queries, external-message broadcasts, and shard public-overlay identifiers | LiteAPI requests or the generic overlay peer manager |

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

## Overlay And Mempool Ownership

The transport/application split is explicit and has no compatibility aliases:

| Location | Owns | Does not own |
| --- | --- | --- |
| `crate::overlay` | ADNL UDP and QUIC overlay sessions, session factories and lookups, bootstrap and discovery, peer growth, `PeerManager`, `OverlayPeerPool`, peer quality, `PeerStore`, inbound query answering, membership gossip, capabilities, DHT address publication, STUN, endpoint registries, packet pre-deduplication, protocol statistics, and signed simple/FEC broadcast encoding | External-message validation, envelopes, or application events |
| `crate::overlay::OverlayNodeBuilder` | Transport/discovery/peer-store configuration and construction of `OverlayClient` or `OverlayServer` | External-message configuration or handlers |
| `crate::mempool` | External-message BoC envelope checks, TL-B validation, age-aware deduplication, lazy events, local echo, and submission/receipt through an already-built `OverlayClient` | Starting transport, discovery, peer management, broadcasts without a client, DHT publication, or overlay configuration |

`OverlayClient` exposes packet and peer-status subscriptions, the peer manager,
generic signed and raw broadcasts, discovery/protocol statistics, peer counts,
and shutdown. `OverlayServer` owns server-only maintenance tasks such as DHT
address publication and exposes its underlying client. `MempoolServer::new`
subscribes to the supplied client before spawning its receiver tasks and
returns a shared `MempoolClient` for submissions.

The `overlay-config` feature is the only overlay feature that downloads global
network configuration. `mempool` depends on `overlay` for transport traits but
does not imply `network-config` or `overlay-config`; callers select bootstrap
configuration explicitly.

## Plumtree Runtime

Plumtree is the public-overlay broadcast path. `OverlayNodeBuilder::plumtree`
is the public configuration point; the shared `PlumtreeHandle` and the
`*_with_plumtree` transport factories stay crate-private. UDP and QUIC sessions
share one handle per overlay, so `subscribe_packets()` observes only decoded
application payloads. `IHave`, `Prune`, `Useful`, statistics pushes, and repair
queries are consumed internally.

The signed simple/FEC broadcast implementation and
`MempoolClient::send_external` remain the legacy external-message path. They
send a single overlay object directly to connected peers and do not participate
in Plumtree eager feedback, duplicate pruning, or repair. Generic public-overlay
payloads should use `broadcast_plumtree_simple` or `broadcast_plumtree_fec`.

Runtime invariants:

- The simple tree is index `0` with part index `0`. FEC part `i` uses tree
  `i + 1`; `tree_slots` must exceed `parts`, and both indices are bounds-checked
  before signature or certificate work.
- For a simple broadcast, the signed `data_size` is the full payload length.
  For FEC, it is the RaptorQ symbol length and the broadcast id is
  `SHA256(TL(overlay.broadcastPlumtreeFecId))` over flags, full-data hash,
  full-data size, and part size.
- Signatures cover the dedicated `...ToSign` TL constructors. Timestamps must
  be finite and within 20 seconds of local time; payloads are capped at 16 MiB.
  Certificates enforce expiry, size, and the FEC-permission flag. Empty
  certificates are accepted for public-overlay traffic; private-overlay
  authorized-key sets are not modelled.
- The `AnySender` flag is preserved and signed like any other flags value.
  Eligibility still binds the payload to the signing source key and certificate;
  the flag does not bypass signature checks.
- Delivery is exactly once per broadcast id. Duplicate payload parts produce a
  `Prune` without application publication. A changed payload under an existing
  id is a conflict and is rejected.
- At most 1,024 missing FEC parts and 512 active repair queries are tracked.
  Repair requests wait for the configured timeout unless the tree has no eager
  slot. A repair response must match the requested key and a part is served only
  after an `IHave` to that peer while eager capacity remains. Delivered and
  broadcast state expires after 20 seconds.
- A node with `receive_broadcasts` disabled does not publish or forward incoming
  Plumtree payloads, but its outbound broadcast methods remain available. This
  is internal state semantics for `DoNotReceivePlumtreeBroadcasts`; there is no
  public private-overlay authorization model.

Each overlay owns a Tokio `Mutex` around the Plumtree state. State transitions,
signature checks, duplicate checks, and action planning happen under that lock;
network sends, repair queries, and packet publication happen after it is
released. Queued actions are drained by the overlay's Plumtree task, and repair
queries are answered by the shared inbound dispatcher for both UDP and QUIC.

The loopback tests cover eager relay, control-frame hiding, and lost-FEC-part
repair over both transports. A client-only node cannot be discovered as a live
relay because it does not publish an address. Use `start_server` and a reachable
UDP endpoint for relay work; public-overlay delivery remains best effort.

## Wire Invariants

- TL constructor identifiers are 32-bit little-endian values.
- The distinction between boxed objects and bare field types is part of the
  schema. For example, `AdnlIdShort` is bare inside `adnl.packetContents` and
  needs its constructor prefix only when serialized as a top-level object.
- LiteAPI common identifiers are encoded with their TON `int`, `long`, and
  256-bit field widths. Do not widen or normalize them at the wire boundary.
- Overlay query framing includes the overlay id before the inner query.
  Overlay broadcasts are raw overlay objects and must not be wrapped as
  `overlay.unicast`; only unicast payloads use that wrapper.
- A Plumtree repair query uses `overlay.query{overlay_id}` followed by
  `overlay.repairPlumtreePart`; QUIC wraps that query in `quic.query` and the
  answer in `quic.answer`. Repair responses are ordinary simple/FEC broadcast
  objects, never `overlay.unicast`.
- TL-B decoding is exact at schema boundaries. Trailing bits or references are
  an error unless the schema explicitly owns them.
- Ordinary cells hold at most 1,023 data bits and four references. Builders and
  slices reject overflow rather than truncating it.
- Exotic cells and BoC roots retain their original bytes where the typed layer
  does not yet model them. Generic BoC decoding is not a proof.
- A representation hash is computed over the canonical cell representation,
  not over the serialized BoC envelope. Proof helpers compare the roots they
  actually decoded, and callers must opt into the relevant checks.

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
- Contract helpers expose `ContractError::Decode(TvmError)`,
  `ContractBuildError::InvalidCodeBoc(TvmError)`, and
  `ContractBuildError::StateInitSerialization(TvmError)`. Stack conversion has
  its own typed error rather than being stringified.
- LiteClient BoC helpers return `liteclient::boc::LiteBocError` for TVM, TL-B,
  root-hash mismatch, account mismatch, non-standard account addresses, and
  account-state mismatch. `LiteError::Boc(LiteBocError)` and
  `LiteError::Decode(TvmError)` preserve those sources at the client boundary.
- The library must not use `anyhow` in production code or the normal dependency
  graph. `anyhow` remains available to tests and the CLI binary, where
  application-level context is useful.
- `overlay::OverlayError` owns bootstrap, discovery, session, peer-store,
  broadcast, and overlay-configuration failures. `mempool::MempoolError` owns
  external-message validation/decoding, submission, event-queue, and
  mempool-configuration failures. Overlay transport errors are converted only
  at the mempool submission boundary and never re-exported as overlay types.

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

- `PeerStore::load` reads peer state with `tokio::fs::read`.
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
- The overlay endpoint registry and packet pre-deduplication cache use
  standard-library mutexes only for short map updates with no await point.
  Peer and session maps use Tokio locks because their critical sections cross
  async receive and fan-out operations.
- `MempoolServer` creates packet and status subscriptions synchronously before
  spawning its forwarding tasks, so startup packets cannot race the receivers.
  Its bounded event queue applies backpressure to inbound packets while local
  echoes use a non-blocking send and are dropped when the queue is full.

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

Live operations cross a trust boundary: ADNL transport, LiteAPI queries,
overlay membership, mempool broadcasts, and message submission. The application
selects the network, liteserver, public keys, timeout and retry policy, and
proof-validation policy. A decoded response is not automatically a verified
response, and a successful submission is not inclusion or finality proof.
Proof helpers verify only the relations they document over the roots they
actually decoded.

The mempool server observes pending external-message broadcasts. It is a low-latency
observation path, not a canonical transaction index or inclusion proof.

## Verification

Run the complete local gate from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p tonutils-schema-gen -- check
RUSTDOCFLAGS="-D warnings" cargo doc -p tonutils --all-features --no-deps
```

The public-mainnet Plumtree smoke test is ignored by default because it needs
live network access and public-overlay traffic:

```bash
cargo test -p tonutils --all-features \
  overlay::plumtree::integration_tests::mainnet_public_overlay_receives_a_completed_plumtree_payload \
  -- --ignored --nocapture
```

Check each standalone feature surface:

```bash
for feature in adnl tl tvm tlb network-config metadata liteclient contracts wallet overlay overlay-config mempool crc serde; do
  cargo check -p tonutils --no-default-features --features "$feature"
done
```

Inspect the normal dependency graph when dependency features change:

```bash
cargo tree -p tonutils -e normal
```

The normal `tonutils` graph must not contain `anyhow`.
