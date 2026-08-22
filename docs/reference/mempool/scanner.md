# Mempool scanner design

`tonutils-mempool` reports what an overlay receive path has seen. It does not
claim that a message is accepted by a validator and it does not replace
LiteServer block/indexing queries.

## Events and lifetime

- `ExternalMessage` means **Seen**: the envelope passed the configured fast
  checks and its SHA-256 hash was not observed in the scanner lifetime.
- `Included` remains a compatibility helper for callers that already perform
  correlation; inclusion tracking is not part of the scanner startup path.
- `PeerStatus` reports transport lifecycle only.
- A message that was deduplicated is not emitted again; an unknown final state
  remains **Unknown**, not `Included`.

The raw BoC is held in `Arc<[u8]>`, so event consumers and broadcast peers can
share ownership without copying the payload. The bounded event queue provides
backpressure. Deduplication is sharded by the first hash byte and evicted by
the configured TTL and bounded shard capacity. `MempoolMetrics` exposes
accepted, duplicate, rejected, broadcast-failure, and rate-limited
invalid-warning counters.

## Fast and slow paths

The fast path checks size, minimum envelope length, and (by default) the BoC
magic `b5ee9c72`, validates an external `Message` by default, hashes the raw
bytes, inserts the hash into a shard, and publishes immediately.
`MempoolConfig::validate_message` can be disabled only for structural/raw
transport tests. `LazyExternalMessage` decodes the stored BoC through
`tonutils-tvm` and `tonutils-tlb` on demand. Consumers can persist
observations, query LiteServer, and call `mark_included` independently.

## Bootstrap and startup

`MempoolScannerBuilder::start` merges explicit `SeedPeer` values, caller-owned
`ConfigGlobal` liteserver endpoints, optional raw global-config JSON, and the
mainnet or testnet global-config URL. HTTP downloading is performed by the
builder, while `tonutils-network-config` remains an offline parser. Duplicate
`(peer, address)` pairs and malformed socket addresses are rejected before the
overlay manager starts; no validated peer is a startup error.

`MempoolScannerBuilder::session_factory` is the explicit transport boundary:
applications provide a factory that performs the canonical ADNL and
overlay-specific handshake and returns an authenticated `OverlaySession`.
For the native UDP path, `native_udp` wires those pieces together; the lower
level `direct_factory`, `channel_factory`, and `udp_dht_lookup` helpers remain
available when applications need custom lifecycle policy.
`native_udp_seeds_only` is the minimal mode: it connects only explicit
`SeedPeer` values and does not perform DHT expansion.
Its overlay adapter accepts TON's `overlay.message` prefix followed by
`tonNode.externalMessageBroadcast` and publishes the nested external BoC.
When present, startup connects every validated discovery result concurrently
and fails if all session attempts fail. Without a factory, startup still builds
the bounded scanner for dependency-injected or offline session management.
Canonical DHT/overlay queries and QUIC remain outside this crate until their
upstream wire fixtures are available.

## Reference comparison

The architecture is informed by
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool): receive from
multiple peers, deduplicate, broadcast, and correlate inclusion. The Rust API
uses `Stream` and does not expose a WebSocket compatibility layer.

## Current gaps

Canonical external-message envelope fixtures, protocol-specific overlay
constructors, ADNL channel negotiation, and LiteServer inclusion tracking still
need upstream schemas, fixtures, and live-network tests. The ignored tests in
`tests/live.rs` validate the mainnet/testnet seed environment contract only;
they do not claim a live network connection.
