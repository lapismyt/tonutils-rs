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
`SeedPeer` values and does not perform DHT expansion. Both native UDP modes
install a DHT `address` publisher (`address_publisher`): one round before the
first peer-growth round and then every `PUBLISH_INTERVAL` (10 minutes)
resolves this node's externally reachable UDP address (`external_address`,
`TON_MEMPOOL_EXTERNAL_ADDRESS`, or a route probe that self-skips behind NAT)
and stores the signed `dht.value` on the closest DHT nodes, so third parties
can resolve and ping this node; see
[DHT address publishing](../network/dht-address-publishing.md).
For this native connector, `SeedPeer.peer` must be the raw 32-byte Ed25519
public key; `SeedPeer::from_public_key` avoids confusing it with an ADNL hash.
Its overlay adapter accepts TON's `overlay.message` prefix followed by
`tonNode.externalMessageBroadcast` and publishes the nested external BoC.
The strict live test is enabled with repository variables
`TON_MEMPOOL_LIVE_SEEDS` (semicolon-separated `KEY@IP:PORT` entries) or the
legacy single-peer pair `TON_MEMPOOL_LIVE_SEED` and
`TON_MEMPOOL_LIVE_PEER_KEY`, plus `TON_MEMPOOL_LIVE_OVERLAY_ID`; it skips only
when no seed configuration is present.
When present, startup connects every validated discovery result concurrently
and fails if all session attempts fail. Without a factory, startup still builds
the bounded scanner for dependency-injected or offline session management.
Canonical DHT/overlay queries are implemented for the native UDP path and for
QUIC: `native_udp` and `native_quic` both install a seed discovery lookup when
`dht_overlay_key` is set (`udp_overlay_lookup` and `quic_overlay_lookup`).

## Membership and liveness behavior

Each registered session sends `overlay.getRandomPeers` once every
`KEEPALIVE_INTERVAL` of 10 seconds while it is idle. Upstream nodes drain their
bounded pending-peer set at one node per second, so a periodic re-announcement
is what eventually produces an incoming `overlay.ping`; answering it is what
makes the peer treat this node as a verified neighbour that receives pushed
broadcasts. A one second cadence produced tens of thousands of queries per run
without improving admission, because the peer's drain rate - not our query
rate - decides when this node is pinged.

After bootstrap, `native_udp` also installs a peer-growth lookup
(`udp_peer_growth`, `PEER_GROWTH_INTERVAL` of 10 seconds, pytoniq's
`OverlayManager.get_more_peers` cadence). Each round picks the next known
member, asks it for `overlay.getRandomPeers`, validates the returned
`overlay.node` records, resolves their DHT `address` values, and opens sessions
for the candidates until `overlay_max_peers` (default 30, pytoniq's
`max_peers`) is reached. Growth also re-announces this node, refreshing the
`version` of its signed record in that peer's queue.

An `overlay.node` record carries no address, so a member reached only
transitively becomes connectable only through its DHT `address` value. Growth
resolves those values with `dht.findValue` the way pytoniq's
`DhtClient.get_overlay_node` does: through the **bootstrap DHT resolver
seeds** (`udp_peer_growth_with_resolvers`, a shared list
`MempoolScannerBuilder::start` fills from the resolved bootstrap seeds), and
only then through the answering member's own session as a fallback.

Resolving against the resolver seeds rather than the member alone is what
makes growth work on a live overlay: an overlay member may be a client that
never answers `dht.findValue`, while the config's bootstrap nodes are DHT
nodes that do. A member that answers only `overlay.getRandomPeers` therefore
costs one bounded hop instead of the whole lookup. Each candidate's `findValue`
follows `valueNotFound` closer nodes (see `resolve_address_on_session`), so a
first hop that is not responsible for the key still converges on the nodes
that are, rather than discarding the closer nodes and reporting no candidate.

Candidates are resolved concurrently, each with its own deadline, because a
few slow nodes otherwise consume the whole round and return no candidates at
all.

Answers to `overlay.getRandomPeers` are not limited to this node. Every
verified member harvested from an answer is kept in the shared
`OverlayMemberCache` of `overlay_factory` and up to five of them are gossiped
back with this node's own record, mirroring upstream
`OverlayImpl::send_random_peers`. Records are validated with
`valid_overlay_node` (overlay id, 600 second `overlay_peer_ttl`, signature)
on insertion and re-validated before being advertised, so the cache never hands
out expired or foreign members. The cache is gossip citizenship only: it does
not change whether this node itself is admitted to a peer's pending set.

Overlay seed discovery resolves the DHT `address` record of every candidate
concurrently. Serializing those lookups let a few slow nodes consume the whole
discovery deadline and produced empty discovery results under load.

`OverlaySession::last_activity` lets the pool keep a session whose
`receive()` is still consuming protocol messages (ADNL answers, overlay
queries, channel negotiation) past `OverlayConfig::peer_idle_timeout`;
sessions without a reportable activity stamp keep the previous hard-deadline
behavior.

Diagnostics for live runs come from `protocol_stats()` (protocol counters such
as `queries_received`, `random_peers_queries_sent`, and `membership_answers`)
and `discovery_stats()` (seed count, discovered peers, session attempts).

## Reference comparison

The architecture is informed by
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool): receive from
multiple peers, deduplicate, broadcast, and correlate inclusion. The Rust API
uses `Stream` and does not expose a WebSocket compatibility layer.

## Current gaps

The remaining acceptance gap is `overlay_packets > 0` on a real overlay
(`TODO.md`): the ignored strict test `configured_seed_delivers_valid_external_message`
runs only when `TON_MEMPOOL_LIVE_SEEDS` (or the legacy single-peer
`TON_MEMPOOL_LIVE_SEED` and `TON_MEMPOOL_LIVE_PEER_KEY`) and
`TON_MEMPOOL_LIVE_OVERLAY_ID` are provided. A NAT'd local host cannot publish
its DHT `address` value by design, so delivery is validated on a public-IP
GitHub Actions runner (`.github/workflows/live-tests.yml`) rather than from a
private network. LiteServer inclusion tracking is intentionally out of scope.
