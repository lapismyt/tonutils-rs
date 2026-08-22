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

`MempoolScannerBuilder::start` merges explicit `SeedPeer` values and optional
raw global-config JSON. HTTP downloading is performed by the builder, while
`tonutils-network-config` remains an offline parser. `ConfigGlobal` is a
LiteServer-only model and is not treated as an overlay seed. Duplicate
`(peer, address)` pairs and malformed socket addresses are rejected before the
overlay manager starts; no validated peer is a startup error.

`MempoolScannerBuilder::session_factory` is the explicit transport boundary:
applications provide a factory that performs the canonical ADNL and
overlay-specific handshake and returns an authenticated `OverlaySession`.
For the native UDP path, `native_udp` wires the session and join query
together; `dht_overlay_key` or `native_udp_for_shard_public` additionally
enables DHT overlay-node resolution. The lower-level `direct_factory`,
`channel_factory`, `udp_dht_lookup`, and `udp_overlay_lookup` helpers remain
available when applications need custom lifecycle policy.
`native_udp_seeds_only` is the minimal mode: it connects only explicit
`SeedPeer` values and does not perform DHT expansion.
For this native connector, `SeedPeer.peer` must be the raw 32-byte Ed25519
public key; `SeedPeer::from_public_key` avoids confusing it with an ADNL hash.
Its overlay adapter accepts TON's `overlay.message` prefix followed by
`tonNode.externalMessageBroadcast` and publishes the nested external BoC.
The strict live test is enabled with repository variables
`TON_MEMPOOL_LIVE_SEED`, `TON_MEMPOOL_LIVE_PEER_KEY`, and
`TON_MEMPOOL_LIVE_OVERLAY_ID`; it skips only when those variables are absent.
When present, startup connects every validated discovery result concurrently
and fails if all session attempts fail. Without a factory, startup still builds
the bounded scanner for dependency-injected or offline session management.
Canonical DHT/overlay queries are implemented for the native UDP path. QUIC
remains outside this crate by design.

## Reference comparison

The architecture is informed by
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool): receive from
multiple peers, deduplicate, broadcast, and correlate inclusion. The Rust API
uses `Stream` and does not expose a WebSocket compatibility layer.

## Current gaps

The remaining acceptance gap is a configured real overlay seed: the ignored
strict test validates external-message delivery only when
`TON_MEMPOOL_LIVE_SEED`, `TON_MEMPOOL_LIVE_PEER_KEY`, and
`TON_MEMPOOL_LIVE_OVERLAY_ID` are provided. LiteServer inclusion tracking is
intentionally out of scope.
