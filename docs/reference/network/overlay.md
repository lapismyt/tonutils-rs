# Overlay protocol design

This page records the boundary between the currently implemented peer
management primitives and the TON wire protocol still requiring upstream
fixtures. It is not a claim that a generic ADNL TCP connection is an overlay
connection.

## Crate mapping

- `tonutils-adnl` owns ADNL framing, handshake, and transport primitives.
- `tonutils-overlay` owns `OverlayId`, `PeerId`, `RoutingMetadata`, bounded
  packet queues, signed discovery records, peer scores, transport-neutral
  sessions, and peer status events.
- `DiscoveryConfig::discover` applies a timeout-bounded DHT-first callback and
  deterministic seed fallback.
- `tonutils-adnl` owns the opt-in UDP direct packet, channel packet, and
  authenticated `AdnlUdpSession` primitives, plus the `QuicSession`
  transport behind the `quic` feature.

## Fast-path model

`OverlayPeerPool` rejects packets over `max_packet_size` before enqueueing,
requires a registered peer, and applies bounded backpressure through Tokio's
multi-producer channel. Consumers can keep a persistent receive loop separate
from application callbacks. `RoutingMetadata` records overlay, peer, receive
time, and hop count without decoding the payload.

## Publishing external messages

`MempoolScanner::send_external` publishes a raw external message BoC
straight onto the overlay every session is connected to.
`MempoolScannerBuilder::start` wires an `OverlayBroadcastPeer` signed with
the builder's local ADNL key, so `native_udp`/`native_quic` is all the
setup the send path needs.

The wire construction matches upstream
`FullNodeShardImpl::send_external_message`
(`validator/full-node-shard.cpp`): the BoC is wrapped in a serialized
`tonNode.externalMessageBroadcast` (`0x3d1b1867`), and the wrapper is
broadcast as

- a signed simple broadcast (`overlay.broadcast`, `0xb15a2b6b`) when the
  wrapper fits `Overlays::max_simple_broadcast_size()` (768 bytes) - one
  datagram per peer, signature over
  `overlay.broadcast.toSign{sha256(overlay.broadcast.id{src, sha256(data),
  flags}), date}` (`BroadcastSimple::create`), or
- signed RaptorQ symbols (`overlay.broadcastFec`, `0xbad7c36a`) for
  everything larger: 768-byte symbols (upstream
  `BroadcastFecActor::symbol_size_`), the raw symbol bytes in `data` and
  the symbol id in `seqno` - the wire format `BroadcastsFec::process_broadcast`
  (`overlay/broadcast-fec.cpp`) decodes - each part signed over
  `overlay.broadcast.toSign{sha256(overlay.broadcastFec.partId{sha256(
  overlay.broadcastFec.id{src, sha256(fec.Type), data_hash, size, flags}),
  sha256(symbol), seqno}), date}` (`BroadcastsFec::send_part`).

Both paths propagate: upstream receivers verify the signature and re-send
the broadcast to their neighbours, so a published message spreads without
sender-side retry. The signed object is serialized once and the identical
bytes are fanned out to all UDP and QUIC sessions in parallel
(`PeerManager::broadcast_object_in_order`), which sends the raw object
after the `overlay.message` header - sessions ranked by delivery quality
first, the rest right behind them. Overlay broadcasts must ride as raw
objects: upstream's `OverlayImpl::receive_message` dispatches on the object
constructor, so the `overlay.unicast` wrapper that `OverlaySession::send`
adds would deliver broadcast bytes to the plain-message callback instead
of the broadcast machinery.

Sends never wait on a session's receive loop. A session receive task holds
its session's mutex while `receive()` waits for - and consumes protocol
traffic inside - the next inbound payload, so a send through the shared
session would stall on any member that does not push this node custom
broadcasts: until the next broadcast packet from that member or until
`peer_idle_timeout` (60 s) expires, with one such member delaying a whole
fan-out join. Both transports are internally synchronized - an ADNL/UDP
session is `Clone` over a shared `Arc`, a QUIC session shares an `Arc`
connection - so each exposes an `OverlaySessionSender` handle
(`AdnlUdpOverlaySender`, `QuicOverlaySender`) whose `send`/`send_object`
frame and transmit by `&self` without the mutex. `SharedSession` stores the
handle next to the session, and `broadcast`, `broadcast_object`, and
`broadcast_object_in_order` prefer it; sessions without a handle (mocks,
custom transports) fall back to the mutex path unchanged.

On the inbound side, the two receive paths consult a shared
raw-payload pre-dedup cache (`PacketPreDedup`) before the TL parse and
ed25519 verification: relayed copies of one broadcast are
byte-identical (relays forward the origin's signed object verbatim),
so the first sighting of a `sha256(payload)` hash records it and every
later copy is dropped for the cost of one hash. The cache is shared by
every session of one factory (flood copies arrive from different
peers), bounded at 4096 entries with a 300 s TTL that mirrors
`MempoolConfig::dedup_ttl`, and evicts expired entries first, then the
oldest survivor by a monotonic insertion sequence. Suppressed copies
count into `ProtocolStats::payload_duplicates`. This mirrors upstream's
early dedup (`BroadcastFecPart::run` rejects duplicate part ids before
`run_checks`; `BroadcastSimple` caches a broadcast id with its checked
state) and stops a flood from scaling signature verifications linearly
in the number of copies; the scanner's own `accept_fast` dedup still
runs last for anything that gets past the cache.

Upstream paces FEC symbol emission at four symbols per 10 ms tick
(`BroadcastFecActor::alarm`) and emits `(size / 768 + 1) * 2` symbols
per broadcast - the `symbols_count` source symbols plus one repair
symbol each. This implementation follows both: the fan-out sends four
parts per 10 ms tick with the first four leaving immediately, so
typical external messages (up to four symbols) finish within one tick,
and the repair symbols let a receiver that loses any single datagram
still reconstruct from the remaining parts instead of waiting for a
retransmission that never comes (the sender does not resend a
broadcast). Both halves report into `ProtocolStats` so a live run can
explain the FEC traffic without logs: `fec_parts_sent` /
`fec_repair_parts_sent` count the fan-out, `fec_symbols_accepted` /
`fec_repair_symbols_received` count parts fed to a decoder,
`fec_decoded` counts verified reconstructions, and
`fec_param_rejections` counts parts refused on the hardened bounds
(unsupported FEC type, invalid reassembly parameters, excessive symbol
id).

The wire `seqno` numbers source symbols `0..symbols_count` and then
repair symbols densely (`symbols_count + i`). The raptorq crate's
decoder instead reserves `[symbols_count, K')` for the zero-padded
extended symbols of the RFC 6330 block and numbers repair symbols from
`K' = extended_source_block_symbols(symbols_count)`, so both receive
paths (`udp_session`, `quic_session`) shift wire repair ids up by
`K' - symbols_count` before feeding the decoder - the same shift
upstream's `td::fec::raptorq::Decoder::add_symbol` applies
(`symbol.id += K_padded - K`). Source ids pass through unchanged, so
source-only part streams keep decoding exactly as before. As a side
effect of the redundancy, a late repair-part stream can assemble a
second time after the first reassembly; the duplicate reaches
`accept_fast` dedup and is dropped there.

## Peer delivery quality and eviction

`MempoolScanner` records how each session behaves on the external-message
flood in a `PeerQualityRegistry` (`peer_quality.rs`):

- the peer that delivers a hash first earns first-delivery credit;
- a duplicate delivered within `late_threshold` (default 500 ms) of the
  first sighting is an on-time delivery; at or after the threshold it
  earns a late penalty - but only for peers that were connected at the
  first sighting, since a peer that joined afterwards could not have
  relayed the message earlier;
- a peer connected at the first sighting that delivers nothing within
  `miss_window` (default 3 s) earns a miss penalty.

Scores decay exponentially with `score_halflife` (default 60 s) on every
sweep, so stale behavior fades. Local sends (`send_external`, the zero
`PeerId`) are never attributed: opening a window for this node's own
broadcast would charge misses to peers for not echoing it back, which
upstream flooding does not guarantee. Attribution runs inside the dedup
shard section, so a concurrent duplicate of a hash always finds an open
window and is never charged a miss for a delivery it just made.

Two background tasks ride on the registry:

- the sweep task (every `sweep_interval`, default 1 s) refreshes the
  connected snapshot used for miss attribution and closes expired windows;
- the eviction task (`quality_eviction(true)`, `churn_interval` default
  60 s) evicts the worst non-pinned peers while the managed connection
  count exceeds `overlay_max_peers`. Only peers with at least
  `min_observations` (default 10) observations are candidates - on thin
  evidence the scan stays over the limit rather than dropping a peer on a
  noisy score. Eviction is opt-in; quality metrics and ranked sends are
  always collected.

Pinned peers (`pinned_peers`, e.g. configured seeds or validators) are
merged into the bootstrap set with reconnect support, excluded from
`overlay_max_peers` (they are a separate, guaranteed set), and never
evicted - even with a terrible score. `pinned_max_peers(Option<u32>)`
caps the pinned set itself, independently of `overlay_max_peers`.

Every send ranks the connected sessions once per call
(`PeerQualityRegistry::rank`, score descending, reused for all FEC parts)
and `PeerManager::broadcast_object_in_order` polls the ranked sessions
first, so the best relays start their datagrams while slower sessions are
still being written. Background peer growth runs only while the scanner is
idle: a round is skipped when the event queue is below 25 % free capacity.

## QUIC member discovery

QUIC discovery does not dial the bootstrap seeds. Upstream runs QUIC only in
`validator-engine`: `validator/full-node-fast-sync-overlays.cpp` starts a
`QuicServer` unconditionally ("Enable quic server even if use_quic is not
set") on `adnl_udp_port + NODE_PORT_OFFSET` (`quic/quic-sender.h`, offset
1000), while the global config's `dht.static_nodes` are `dht-server`
deployments whose sources contain no QUIC listener at all, so probing them
at `port + 1000` can only time out.

`MempoolScannerBuilder` therefore owns an `OverlayMemberEndpoints` registry
that the UDP path fills from three sources:

- every DHT `address` record resolved during seed discovery and peer growth,
  including an advertised `adnl.address.quic` entry, which upstream
  `QuicSender::get_ip_address` prefers over the derived port;
- every member that answers `overlay.getRandomPeers` with at least one node
  record valid for the overlay - the growth path and the QUIC path alike;
- the source route of a session whose peer answers `overlay.getRandomPeers`
  (`AdnlUdpSession::remote_addr`, which follows NAT rebinding through the
  transport's `adopt_address`).

`quic_overlay_lookup_with_endpoints` probes the registered members - sorted
by peer id so the subset is stable across runs, capped at `max_records` -
and falls back to the bootstrap seeds only when the registry is empty, which
is logged. Each probe emits one line:

```text
quic probe: member=<hex> target=<ip:port> source=<advertised|udp+1000> result=<connected|timeout|error: ...> via=<dht.findValue|getRandomPeers|session>
```

Whether mainnet full nodes answer QUIC from a third-party source is
settled for GitHub Actions as of run 38237835993 (2026-10-12): they do
not answer at all. Raw Version Negotiation probes received
`vn_replies=0` from all 13 member targets while `1.1.1.1:443` and
`8.8.8.8:443` answered in the same phase, and the same hosts had
answered ADNL UDP `overlay.getRandomPeers` from the same runner minutes
earlier on their base ports, so runner egress - including high UDP ports
to those exact hosts - is ruled out: the `adnl_udp_port + 1000` targets
are silent before any handshake, either because nothing listens there
(builds predating the QUIC merge) or because datagrams are dropped
before TLS, which transport silence cannot distinguish. The live test
stays in CI to catch nodes that upgrade to listening builds, and the TLS
handshake itself - ALPN, short-id SNI, and raw public keys in both
directions - is specified in [QUIC TLS and wire profile](quic.md).

## Failure modes

`PacketTooLarge`, `UnknownPeer`, and `QueueClosed` are structural/operational
failures. A disconnected peer produces `PeerStatus::Disconnected`; the
`PeerManager` runs independent receive loops, removes failed sessions, and
updates a coarse score. No status implies proof that a peer is honest or that
a packet was included in a block.

`OverlayConfig::peer_idle_timeout` bounds one `receive()` call. Because a
session consumes protocol messages inside `receive()` without completing it,
the pool consults `OverlaySession::last_activity` when the deadline expires
and keeps a session whose inbound traffic is newer than the timeout; the
default implementation returns `None` and preserves the hard deadline.

A session's hand-off to the bounded pool queue
(`OverlayPeerPool::ingest`) carries its own bound: after
`INGEST_WAIT_TIMEOUT` (200 ms) the packet is dropped, the drop is counted
in `OverlayPeerPool::ingest_drops` (surfaced as `MempoolMetrics::queue_full_drops`
by the scanner's receiver workers), and the loop goes straight back to
polling `receive()`. Without that bound a stalled consumer - a full
event queue, one slow BoC parse - would freeze the loop that answers
inline `overlay.ping` and `overlay.getRandomPeers` queries, and peers
drop a member after four unanswered pings, so a receive-loop stall
erodes membership in both directions. Drops are the intended trade-off:
the flood a saturated queue implies carries redundant copies. The
consumer side runs `INGEST_WORKERS` (4) drain workers: dequeueing stays
serialized by the pool's receiver lock while validation, hashing and
dedup run in parallel; packet ordering across workers is not observable
because broadcasts are idempotent under dedup and queries never ride
this queue.

## Upstream peer admission rules verified from source

All of the following come from `overlay/overlay.cpp`, `overlay/overlay-peers.cpp`,
`overlay/overlay-id.hpp`, and `overlay/overlays.h` in `ton-blockchain/ton`.
They close the gap between "a peer answered our query" and "a peer pushes
broadcasts to us".

- **`overlay.getRandomPeers` carries `peers:overlay.nodes`, not an overlay id.**
  `OverlayImpl::process_query(overlay_getRandomPeers&)` starts with
  `add_peers(query.peers_, /* verified = */ false)` *before* it answers. That
  call - not the answer - is what queues this node in the peer's
  `pending_peers_`, so an outgoing query with an empty or missing `peers` list
  makes a node permanently invisible. `send_overlay_get_random_peers_with_id`
  therefore always includes this node's own signed record.
- **Acceptance of an unverified node has six gates**, all of which drop
  silently: `receive_peers_rate_limiter_` of `RateLimiterWindow{10.0,
  10 * nodes_to_send_ * 2 * 10}` (800 nodes per 10 s), an overlay id mismatch,
  `node.version() + overlay_peer_ttl() < now || node.version() > now + 60`
  with `overlay_peer_ttl() == 600`, this node's own id, an Ed25519 signature
  over the boxed `overlay.node.toSign id:adnl.id.short overlay:int256
  version:int`, and an existing entry already in `peers_` (which only
  `update`s and never re-queues a ping).
- **The pending set is a random-eviction queue.** `max_pending_peers_` is 100;
  overflowing it removes `pending_peers_.get_random()`, so a node that is not
  re-announced can be evicted before it is ever pinged.
- **Draining is one node per second.** The overlay alarm runs every second and
  calls `process_pending_peers()`, gated by
  `process_pending_peers_rate_limiter_{60.0, 60}`, which picks one random
  pending entry, sends `overlay.ping` with a 5 s timeout and 3 attempts, and
  only then calls `add_peer(verified = true)`.
- **Verification is what grants a neighbour slot.** `add_peer(verified = true)`
  always inserts into `peers_`, and into `neighbours_` only while
  `neighbours_.size() < max_neighbours()` (10) and the peer's flags have no
  `DoNotReceiveBroadcasts`; otherwise the node sits in `peers_` and waits for
  `update_neighbours`.
- **`update_neighbours(0)` runs every second** and fills free neighbour slots
  from random live members, while `update_neighbours(2)` - scheduled every
  30-120 s - replaces two random slots. So a freshly verified member is picked
  up quickly when a slot is free, and otherwise depends on the member set size
  for a lottery ticket roughly every 75 s.
- **The `update_neighbours` draw is uniform over the verified member set.**
  Every iteration draws `peer_list_.peers_.get_random()`; when the neighbour
  set is full, the drawn member replaces a uniformly random current slot
  (`overlay/overlay-peers.cpp`). With `max_peers_ = 300` and
  `max_neighbours_ = 10`, one full node's `update_neighbours(2)` call gives a
  pooled member roughly `2 / |peers_|` per 30-120 s window, and the same call
  runs a five-slot draw over `plumtree_neighbours_` whenever
  `max_plumtree_neighbours() > 0`. Verified peers also churn: `del_some_peers`
  runs on every verified add and deletes up to 10 peers past `max_peers_`
  (bad-scored first via a rotating pointer, random otherwise), so membership
  can lapse and be re-verified - a fresh admission ticket each time.
- **Liveness in a peer's table is refreshed by any authenticated packet, not
  only by pongs.** `OverlayPeer::on_ping_result` (`overlay/overlay.hpp`)
  starts every peer alive; a success resets `missed_pings_`, re-stamps
  `last_receive_at_` and keeps `is_alive_` true, and `is_alive_` clears only
  after three misses accumulate while `last_receive_at_` is older than 15 s.
  For `OverlayType::Public` overlays, `check_src_peer` forwards every
  authenticated inbound packet to `on_ping_result(src, true)`
  (`overlay/overlay-peers.cpp`), so the scanner's regular query traffic -
  the 10 s per-session `overlay.getRandomPeers` keepalive included - keeps it
  in every accepting peer's `update_neighbours` draw even while it is not a
  neighbour.
- **Pushing is a per-wave random sample of the neighbour set.** Broadcasts fan
  out through `get_neighbours(propagate_broadcast_to())`, and
  `propagate_broadcast_to()` is 5 in both `BroadcastSimple::run_continue`
  (`overlay/broadcast-simple.cpp`) and `BroadcastFec::distribute_part`
  (`overlay/broadcast-fec.cpp`); `get_neighbours` delegates to
  `choose_neighbours` (`overlay/overlay-peers.cpp`), a uniform random sample
  without replacement of at most 5 of the at-most-10 `neighbours_`. A holder
  therefore pushes to a verified neighbour with probability about
  `5 / |neighbours|` per broadcast wave, and a peer whose neighbour set was
  already full when it verified this node becomes pushable only through the
  `update_neighbours` lottery above. This makes delivery all-or-nothing and
  the pinger count non-predictive: the delivering run 38594880666 (2026-10-14,
  `386042b`) had 2 pingers, while the non-delivering run 38723511984
  (2026-10-15, `694aa49`) had 5 and received no broadcast at all.
- **An answer is a sample, not a membership proof.**
  `send_random_peers_cont` builds the reply from `announce_self_` plus up to
  `nodes_to_send_()` (4) calls to `get_random_peer(true)` over the peer's whole
  verified member set. `membership_listed` is therefore about `4 / |peers_|` per
  answer - with a 300-member set that is ~1.3 %, so hundreds of answers are
  needed before a listing is statistically expected, and its absence proves
  nothing.
- **A node only gossips outward with a valid certificate.** The alarm's
  outbound `send_random_peers` to a random neighbour and a random peer is
  guarded by `has_valid_membership_certificate()`.
- **There is no query-based broadcast fetch path to fall back on.**
  `overlay.getBroadcastList` is dropped outright by upstream
  (`OverlayImpl::process_query` logs "DROPPING getBroadcastList query"), and
  `overlay.getBroadcast hash:int256` is answered only from the holder's
  local broadcast store when the hash is present
  (`BroadcastSimple::process_query`), so a broadcast hash cannot be learned
  without receiving a push first. The neighbour fan-out above is the only
  inbound broadcast route.

### The two-tier overlay ecosystem

Mainnet mempool overlays are not one uniform population. Live scanner runs
show two classes of peers with different admission behaviour, and the split
lines up with the implementation each peer runs:

- **Upstream full nodes** (`ton-blockchain/ton`) follow the admission chain
  above end to end. They call `add_peers(query.peers_)` on inbound
  `overlay.getRandomPeers`, drain the pending set, originate `overlay.ping`,
  and after the pong mark the node verified. Every live peer that pinged this
  scanner was of this class, and every such peer also echoed
  `recv_addr_list_version` - the marker that it stores a routable
  `adnl.addressList` for us (`AdnlPeerPairImpl::send_packet`).
- **pytoniq-based bots** (the mempool processer fleet, e.g.
  `yungwine/ton-mempool` with `pytoniq.adnl.OverlayManager`) answer queries
  but never echo `recv_addr_list_version` and never originate `overlay.ping`.
  Their membership code (`OverlayManager`, `ShardOverlay`) is a private
  pytoniq extension absent from every public pytoniq release, so its inbound
  `query.peers` handling is not observable; live evidence shows the bots do
  not admit a fresh identity from queries. Bots bootstrap their member lists
  from the DHT instead: `DhtClient.get_overlay_nodes` reads the
  `overlay.<id>.nodes` value (`pytoniq/adnl/dht.py`), and the bot processer
  wires that client into `OverlayManager`.

The DHT announce path is therefore the bot fleet's root of trust, and it is
closed to an ed25519 scanner identity: upstream `dht.updateRule.overlayNodes`
`check_value` (`dht/dht-types.cpp`) requires the value's keyDescription id to
be a `pub.overlay` key, and `pub.overlay` keys are named blobs without
signing material (`keys/keys.hpp` `pubkeys::Overlay`, `EncryptorOverlay`) -
only full nodes hold the paired private form. pytoniq itself cannot store
such values either (`DhtClient.store_value` rejects the `overlayNodes` rule
with `DhtError('currently overlay is not supported')`). This scanner reads
the value for discovery (`lookup.rs`) but cannot and must not write it.

The practical consequence is a bimodal delivery distribution, but the
observed mechanism is narrower than a listing cascade. A bot could in
principle learn this node when a full node samples it into a
`getRandomPeers` answer (`4 / |peers_|` per answer) and then resolve its
DHT `address` value (`DhtClient.get_overlay_node`), but in every recorded
run - the delivering ones included - publishing ended with
`dht address publish: stored on 0 of 4`, so that value never existed, no bot
could resolve it, and the cascade remains a possible path with no positive
live evidence. What the logs show instead is the direct path: a growth or
bootstrap session peer answers a query, originates `overlay.ping`, marks
this node verified after the pong, and pushes broadcasts to it. The
recorded arms: run 38594880666 (2026-10-14, `386042b`) delivered ~80 s into
the wait, seconds after a growth-phase peer pinged it and started pushing
`overlay.message` custom datagrams; run 38558546040 (2026-10-14, `4894e5f`)
delivered at ~295 s after 4313 such datagrams; runs 38579887480 (2026-10-14,
`5fec546`), 38723511984 (2026-10-15, `694aa49`), and 38745254834 (2026-10-15,
`4274f0e`) received none within the full 300 s budget despite normal
membership and pinger counters. The earlier 129-192 s band is superseded by
these arms: timing follows whether any neighbour holder samples this node in
its fan-out, and `dht_pings_received` was 0 in every run while
`membership_listed` stayed 0 until run 38828796572 recorded 4 listings from
one peer; neither counter tracks the push path. A delivery's absence in a
300 s window still proves nothing about this node's reachability.

Roster freeze: every recent run saturated at `registered_peers=65-71` against
the default `overlay_max_peers=64`, so the managed roster froze near the cap
within the first minute, all admission tickets were spent early, and growth
stopped adding fresh full-node contacts. The CI test now runs with
`TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS=256`, `MempoolConfig.growth_interval=3s`,
`churn_interval=3s`, and `quality_eviction(true)` with a pre-wait burst of
four growth rounds, so eviction -> growth round -> new admission ticket
cycles continue throughout the 300 s wait.

Run 38819591750 (2026-10-15, `fb9afc9`, 300 s budget): timeout arm again.
Aggressive growth (`max_peers=256`, `growth_interval=3s`, `churn_interval=3s`,
pre-wait burst of 4 rounds) raised metrics: `registered_peers=94`,
`address_storing_peers=145`, `unique_pingers=9`, `membership_answers=2400`,
`random_peers_queries_sent=3226`. Despite the strongest admission window
yet recorded, `custom_messages=0` and `overlay_packets=0` — no full-node
neighbour holder sampled this node into its `get_neighbours(propagate_broadcast_to() = 5)` fan-out. This confirms the push lottery is the bottleneck:
even with ~9 verified neighbours, the per-wave push probability (~5/|neighbours|)
can yield zero inbound broadcasts for an entire 300 s window.

Run 38828796572 (2026-10-16, `7ae1cbd`, 300 s budget): delivered. The delta
from run 38819591750 is documentation text only - the delivery test code is
identical - and the admission metrics land in the same band
(`unique_pingers=9`, `address_storing_peers=125`, `membership_answers=2724`,
`random_peers_queries_sent=2436`; the first recorded nonzero listing,
`membership_listed=4` from one peer). This arm counted `custom_messages=1`
and delivered at `time_to_message=222.5s`, while the earlier arm counted zero
for its entire window. The pair - red 22:48 UTC, green 00:19 UTC, same code -
is the cleanest recorded evidence that a live delivery outcome is a per-run
result of the upstream neighbour draw and push fan-out above, not a
client-state difference.

Run 38865936964 (2026-10-16, `cbf6b4a`, 300 s budget): timeout arm again
(`custom_messages=0`, `overlay_packets=0`, `registered_peers=89`,
`unique_pingers=10`, `address_storing_peers=131`, `membership_answers=2522`,
`random_peers_queries_sent=3230`) - the third hardened timeout arm, again
zero inbound broadcasts at healthy admission counters. The CI gate therefore
stopped sampling once: the `live-delivery` job now runs
`TON_MEMPOOL_LIVE_DELIVERY_SAMPLES` (default 8) independent samples in
parallel and passes when at least one delivers. With the recorded per-window
rate (3 green of 9 arms, `p ~ 1/3`) a red gate is a `(1 - p)^8 <= ~4%` false
alarm and therefore a real regression signal, while a single-sample gate was
red on about two healthy runs of three.

Run 38909762774 (2026-10-16, `fe0fa8b`, 300 s budget): delivered and failed a
later phase.  Sample 1 delivered at `time_to_message=106.9s`, then died in the
post-delivery non-seed roundtrip check: all six registry members that the phase
drew failed to answer while the manager held live sessions to other members, so
the phase now probes in waves (connected members first, up to three waves of
six, stopping at the first answer).  The gate is the first where every sample
admitted pingers (`unique_pingers=7-14`, `pings_received=26-61`, against the
previous 0-7 band) and the first with reproducible `served_by_dht=true`
(samples 3, 5, 7: `stored on 3 / 3 / 2 node(s)`), so the admission funnel and
the publish path moved while the push lottery kept seven samples at
`custom_messages=0`.

Run 38912409135 (2026-10-16, `b19ce81`, 300 s budget): all eight samples timed
out with `custom_messages=0`, `unique_pingers=5-9`, `pings_received=14-35`,
`address_storing_peers=99-137` and `membership_listing_peers=0`.  The publish
stage, not admission, was the weak arm in this gate: five samples ended at
`no externally reachable address` and one at `stored on 0 node(s)`; only the
remaining two stored and verified.  The STUN agreement check was counting one
observation per hostname and the two Google hostnames resolve to the same
server, so two answers from a single NAT destination could pass as agreement.
A local probe of the public STUN fleet showed Cloudflare, miwifi, sipnet and
blackberry all answering from the same public IP with a *different* mapped
port per server - the signature of a per-destination (symmetric) NAT, whose
mapping is only valid toward the server that observed it.  Agreement now
counts distinct resolved server addresses only, any disagreement from a
distinct destination disqualifies the round, servers that stayed silent are
retried in a second sweep over a wider list, DNS resolution is bounded, a
store round that stored nothing is retried once, and the gate job prints the
per-sample STUN observations into the job log.

Run 38917780756 (2026-10-16, `1a5cf52`, 300 s budget): green, and the first
gate that classified the runner's NAT.  All eight samples published a
`dht.address` value: Cloudflare (`162.159.207.0:3478`) and Google
(`74.125.250.129:19302`) - two distinct destinations - reported the same
mapped port for each sample's shared socket (one port per sample, 5184-5191),
so the runner's mapping is endpoint independent and every sample passed the
distinct-destination agreement rule.  Sample 7 delivered at
`time_to_message=274.8s` and satisfied the gate; samples 1/3/5/6/8 were
stopped once the winner was found.  The two timed-out samples still recorded
healthy admission (`unique_pingers=4/6`, `pings_received=16/20`,
`membership_answers=1174/1240`), and sample 4 recorded the first non-zero
`membership_listing_peers=2`: full nodes now resolve the published address,
admit this node, and list it back in their own `getRandomPeers` answers.

Run 38922689533 (2026-10-17, `f5f4d69`, 300 s budget): green again.  Sample 3
delivered at `time_to_message=269.9s`; all eight samples resolved an
endpoint-independent mapping (Cloudflare plus Google or miwifi reporting the
same port per sample, 17408-17415).  The publish store round split four/four:
samples 2/4/5/7 stored on 2-4 nodes and verified (`served_by_dht=true`), while
samples 1/3/6/8 recorded six `findNode` failures and two enrichment
exhaustions and ended at `stored on 0 node(s)` even after the retry.  The
delivered sample 3 is one of the unpublished ones, so the window's delivery
came from the direct-announce admission path, not from the stored value; the
timed-out samples kept healthy admission (`unique_pingers=2-9`,
`pings_received=7-34`, `membership_answers=1097-1378`).  Both post-fix gates
delivered at `~270s` of the 300 s budget.

Run 39005547863 (2026-10-17, `5861a89`, 300 s budget): red, all eight samples
timed out at the strongest admission counters recorded so far - sample 7 alone
answered `pings_received=64` from `unique_pingers=16` (pongs = pings in every
sample, `custom_messages=0`), sample 6 reached 12 pingers, three samples
recorded `membership_listing_peers=1`, and every sample resolved an
endpoint-independent mapping.  Publication split: samples 7/8 stored on 3
nodes and verified (`served_by_dht=true`; the sample with the most admitted
holders was published and still saw no broadcast), samples 2/3 stored on 2
without a serving responsible node, and samples 1/4/5/6 hit six `findNode`
failures with two enrichment exhaustions and stored nothing even after the
retry.  Nothing in the window's own counters separates this red draw from the
two green ones: the remaining failure mode is the upstream neighbour-slot
lottery - `process_pending_peers_rate_limiter_{60.0, 60}` services one pending
peer per second in random order from a queue capped at 100, and
`update_neighbours(2)` swaps two neighbours plus five plumtree neighbours
every `U(30,120)` s from up to 100 random draws over the holder's peer table -
and no client-side lever reaches it (announce cadence, reach, publish
correctness, and the pong path are all verified working in this run).

Follow-up policy (2026-10-17): the per-sample delivery window is now 600 s
(the `live-delivery` workflow fallback; k=8 unchanged, the
`TON_MEMPOOL_LIVE_TIMEOUT_SECS` repo-variable override is kept).  The
decision followed this red draw: with no client-side lever left against the
upstream neighbour-slot lottery, a longer window raises the per-sample
probability without the CPU and DHT contention a larger sample count would
add - at the recorded ~8 % per 300 s the constant-hazard extrapolation is
~15 % at 600 s, and the k=8 gate green rate moves from ~35-50 % to ~74 %.

Run 39007384341 (2026-10-17, `a018ab3`, 600 s budget): red, and the wider
window paid off on its first run - sample 4 delivered at
`time_to_message=332.7s`, past the old 300 s deadline, then failed the
post-delivery non-seed roundtrip phase with all 18 probes (3 waves of 6)
unanswered.  The other seven samples timed out at the strongest admission
counters recorded so far (sample 2 answered `pings_received=136` from
`unique_pingers=30`, sample 1 `114`/`18`, sample 8 `126`/`14`), and every
sample resolved an endpoint-independent mapping again.  The roundtrip phase
turned out to be the only dial in the tree that opened a second socket with
a foreign keypair: every other path - manager sessions, DHT lookups, address
resolution, growth - resolves through `AdnlUdpTransport::for_node` and
shares the node's single source address and identity, the rule the transport
documents so a peer never stores a dead one-shot port.  Registry members
answer that identity and almost every foreign one goes silent, which is the
0-of-18 outcome.  The phase now probes through the same construction
(`for_node` + `session_for_peer`, so a member with a live session answers its
probe on it): the member's recorded endpoint must carry traffic for this
node's identity, and the endpoint-staleness lottery of the wave ladder is
all that remains.

Run 39008789830 (2026-10-17, `3a791ac`, the phase fix): red on the pure
delivery lottery - all 8 samples timed out at 600 s with healthy counters
(max `unique_pingers=17`, `pings_received=75`; all 8 resolved an
endpoint-independent mapping; three samples recorded
`membership_listing_peers=1`), so the phase fix was not exercised by the
draw.  The classified re-run of the same job then died inside the
classification itself: the runner executes the step with `bash -e`, and a
`field="$(grep ... | ...)"` extraction whose grep finds no match aborts the
whole step, which a split metrics line (interleaved writes from the test's
threads) triggers - header printed, no diagnosis, exit 1.  The script now
disables errexit before any diagnostics, keeps a degraded prefix when the
structured census line cannot be parsed, and uploads the raw per-sample
logs as an artifact on red draws; the failure was reproduced against a
split-line fixture before the fix and cleared after it.  Sixteen 600 s
samples have now delivered once (332.7 s), and the admission counters show
why the rate moves per draw (30 pingers on one gate, 17 on the next), so
the ~15 %/sample window estimate stays an upper bound until more draws
accumulate.

Run 39011060928 (2026-10-17, `aec38f6`, 600 s budget): red - 2 of 8 samples
delivered (the best at `time_to_message=360.2s`), and the delivered sample
failed the post-delivery non-seed UDP roundtrip phase with 0 of 18 probes
(panic at `live.rs:612`).  The phase failure was neither endpoint staleness
nor member-side: the identity fix from 39007384341 dialed through `for_node` +
`session_for_peer`, and `session_for_peer` adopts the requested address on the
shared session unconditionally.  A growth round or probe wave fires all of a
member's recorded endpoints concurrently, so whichever candidate adopted last
- a dead recorded port, five of six in the captured member's list - became
the session's single send address; the manager's keepalives, growth queries,
and every later probe were then addressed to it while the member listened on
its live port, and only queries sent near a re-adopt of the live port were
answered.  The captured member's shape fits exactly: 51 answers spread over
the run, none for a growth query that followed a dead-port adopt, and a lone
exchange to the live port under the same manager-like keepalive stream
answered in 73 ms.  A local replica of the sweep (six concurrent probes plus
the keepalive stream) reproduced the black-hole deterministically: all six
probes timed out and every subsequent keepalive went to the dead port for the
rest of the run.  The fix separates dialing from ownership: a session's
address changes only through a deliberate adopt (`session_for_peer`, used by
the manager, bootstrap, and reconnect paths), dial-only callers get the same
session through `session_for_probe` without repointing it, and probes that
dial one endpoint carry it on the query itself
(`overlay_get_random_peers_to`, `send_overlay_get_random_peers_to`).  The
growth path and the phase now use both, the phase's `for_node` identity
construction is unchanged, and the offline regression test
`tonutils_adnl::adnl::udp::tests::probe_sessions_keep_the_adopted_address`
pins the contract.

### Address routing: channels versus stored lists

A peer needs a routable address for this node only when it has no established
ADNL channel: once a channel is up, pings and broadcasts flow back over the
observed UDP path regardless of what the peer stored. Peers that know this
node only from a record (DHT value or gossip sample) must resolve its DHT
`address` value before they can ping or push. Outbound packets carry an
`adnl.addressList` with an empty `addrs` vector and a `version` stamped at
send time, which upstream accepts as an implicit source address
(`AdnlPeerPairImpl::receive_packet_checked`: an empty list plus a valid
remote address adds the datagram source), so channel peers additionally learn
the address without any DHT lookup.

The observable per-peer signal is the `recv_addr_list_version` echo. The
`tonutils_adnl::adnl::udp::address_storing_peers` counter tracks distinct
peers that ever echoed it, and `ProtocolStats::unique_pingers` /
`ProtocolStats::membership_listing_peers` / `ProtocolStats::dht_pings_received`
split the later admission stages per distinct peer, so a live run can tell
"the peer cannot reach us" from "the peer does not admit us" from "the peer
keeps us in its own list".

A `dht.address` value is published only when the mapping is endpoint
independent: several STUN servers behind distinct destinations must report the
same `ip:port`.  Behind a per-destination NAT no single published address can
serve all peers, so the correct behavior is to publish nothing and rely on the
observed source address every peer learns from this node's own datagrams
(`AdnlPeerPairImpl::receive_packet_checked` accepts the datagram source when
the address list is empty); publishing a per-destination mapping would only
advertise an address that a peer other than the observer cannot deliver to.

A peer id may advertise several endpoints, but the transport keeps one
session per peer id with one send address and one address list version.  That
address is owned by the caller that dials for steady-state traffic
(`AdnlUdpTransport::session_for_peer`: the manager at add time, bootstrap and
reconnect paths); dial-only callers (`session_for_probe`) never repoint an
established session and carry the endpoint they probe on the query itself
(`AdnlUdpSession::overlay_get_random_peers_to`,
`send_overlay_get_random_peers_to`).  The distinction is load-bearing: a
growth round or probe wave fires all recorded endpoints of one member
concurrently, and an adopting sweep leaves the session sending at whichever
candidate adopted last - usually a dead recorded port - so the manager's
keepalives and every later query are black-holed while the member answers on
its live port (CI run 39011060928).

## Reference and unfinished work

The pending-message behavior is compared conceptually with
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool). The Python
project's WebSocket interface is not part of this crate. Canonical ADNL/DHT/
overlay TL constructors, direct UDP live probing, channel create/confirm state
transitions, signed overlay join queries, and transport-to-stream delivery are
covered by checked fixtures and localhost tests.

Incoming overlay queries reach this node wrapped in `overlay.query`
(`0xccfd8443`) or `overlay.queryWithExtra` (`0x94ffc3e9`); the wrapper is
unwrapped, its overlay id is validated against the session overlay, and the
inner query is answered on the original `query_id`. Upstream peer exchange is
the flow `overlay.getRandomPeers` -> bounded pending set -> `overlay.ping` ->
`overlay.pong`; without a successful pending verification, a member never
pushes broadcasts to this node.

The pytoniq nodes that make up the mempool overlay keep peer lists with a
different constructor. `Node.ping` (`pytoniq/adnl/adnl.py`) sends `dht.ping`
(`0xcbeb3f18`) wrapped in `overlay.query` every 60 seconds and removes a peer
after four unanswered pings plus a 15 s guard, so an `overlay.pong`-only peer
is silently dropped from the broadcast fan-out minutes after joining. This
crate therefore also answers `dht.ping` with `dht.pong` (`0x5a8aef81`),
echoing `random_id`, the same answer upstream builds in
`DhtMemberImpl::process_query(dht_ping)` (`dht/dht.cpp`). The wire form is
identical whether the ping arrives wrapped (the pytoniq shape) or bare.

A pushed broadcast is accepted only after its Ed25519 signature verifies, and
the signed payload is the *broadcast id*, not the data hash. Upstream computes
`broadcast_hash = sha256(overlay.broadcast.id{src, data_hash, flags})` where
`src` is the sender's ADNL short id - `sha256(serialize(pub.ed25519 key))` -
zeroed when `BroadcastFlagAnySender()` (1) is set, `data_hash` is
`sha256(data)`, and the signature covers `overlay.broadcast.toSign{broadcast_hash,
date}` (`compute_broadcast_id` and `BroadcastSimple::to_sign` in
`overlay/broadcast-simple.cpp`). A verifier that signs the raw data hash
instead rejects every genuine broadcast, so `OverlayBroadcast::payload_if_valid`
recomputes the id the upstream way; this is what turns arriving
`overlay.broadcast` frames into delivered `overlay_packets`. The
`overlay.broadcast.id` constructor (`0x51fd789a`) is pinned by
`schema_audit.rs` against `ton_api.tl`.

A verified broadcast still carries a *wrapper*, not a BoC. Upstream
`FullNodeShardImpl::send_external_message` (`validator/full-node-shard.cpp`)
and pytoniq `ShardOverlay.send_external_message` both serialize
`tonNode.externalMessageBroadcast message:tonNode.ExternalMessage` into the
broadcast `data` field, and upstream's receiver unwraps it with
`fetch_tl_object<tonNode_externalMessageBroadcast>` in `check_broadcast`.
The deliverable BoC is therefore the nested `message.data`, one level below
the signed payload; handing the wrapper itself to the scanner fails
`validate_envelope`'s BoC magic check (`0x3d1b1867` vs `0xb5ee9c72`) and
counts as a rejected packet. `unwrap_external_message_payload` performs the
same unwrap on the simple-broadcast and `overlay.unicast` paths, matching
what this crate's FEC path already did; payloads that are not that wrapper
pass through so `validate_envelope` stays the single accept/reject point.

A FEC broadcast part carries its *own* signature over the part id, not the
broadcast id. Upstream computes
`broadcast_hash = sha256(overlay.broadcastFec.id{src, sha256(fec.Type),
data_hash, size, flags})` (`compute_broadcast_id` in
`overlay/broadcast-fec.cpp` - the FEC id folds in the serialized
`fec.Type` hash and the full data size, unlike the simple broadcast id),
then `part_hash = sha256(overlay.broadcastFec.partId{broadcast_hash,
sha256(symbol), seqno})` (`compute_broadcast_part_id`), and the signature
covers `overlay.broadcast.toSign{part_hash, date}`
(`BroadcastFecPart::to_sign`). `BroadcastFecPart::run_checks` verifies it
before the symbol is fed to the decoder, so every symbol of a FEC broadcast
is individually signed. The `overlay.broadcastFec.id` (`0xfb3155a6`) and
`overlay.broadcastFec.partId` (`0xa46962d0`) constructors are pinned by
`schemas/ton_api.tl`. On receive, the symbol rides either as upstream's
raw RaptorQ symbol bytes (with the symbol id in `seqno`) or as a
serialized RaptorQ `EncodingPacket` (payload id plus symbol bytes); both
forms reassemble through the same decoder, which is what lets this crate's
own FEC sends interoperate with upstream receivers and vice versa.

Verification is necessary but not sufficient for a push. Upstream
`BroadcastSimple::send` and the Plumtree path both choose
`propagate_broadcast_to_` (5) members out of at most `max_neighbours_` (10),
never the whole member set: `OverlayImpl::add_peer(verified = true)` admits a
member into the neighbour list when it has room, and
`OverlayImpl::update_neighbours(2)` reshuffles two random entries every
30-120 seconds afterwards, drawing uniformly from the peer's member set. The
wait for a first push therefore scales with how many peers completed a
pending verification and with how large their member sets are, not only with
membership. An outgoing node record also has to stay reachable: because
broadcasts are `adnl.message.custom` addressed to the source address the peer
stored from our packets, see `fill_address` in
[ADNL UDP](./adnl-udp.md) for why that address is re-stamped per packet.

`overlay.getRandomPeers` answers carry this node's signed `overlay.node`
record plus up to five members harvested from previous answers. Harvesting
happens in `overlay_inbound::trace_membership`, which validates every returned
record with `valid_overlay_node` (overlay id, `overlay_peer_ttl` of 600
seconds, Ed25519 signature) and stores it in the per-factory
`OverlayMemberCache` shared by all sessions; records are re-validated before
being advertised, so an expired member stops being handed out. This makes this
node a useful gossip citizen but does not affect whether a peer admits it:
admission depends on the peer's pending-peer drain, not on what we answer.

Membership itself comes only from DHT-discovered overlay members. A 600 second
live run against `dht.static_nodes` answered `overlay.getRandomPeers` from every
DHT-discovered member it held and from none of the sixteen config seeds, which
are DHT contacts rather than overlay members, so seeds alone never produce a
verified peer. The member source is the DHT `overlay.<id>.nodes` value - the
overlay's full-node phonebook, which held five members per answering seed in a
local mainnet scout (union across the sixteen config seeds: 20) - and seed
discovery resolves the whole value: one seed's answer is walked up to six
hops, every candidate's `address` record is resolved concurrently
(`ADDRESS_LOOKUP_CONCURRENCY` = 12 in flight) until the round's deadline, and
the lookup's record cap comes from the caller's budget
(`MempoolScannerBuilder::native_udp` passes `overlay_max_peers`), so `start`
registers one session per discovered member within the configured budget
instead of a four-entry-per-seed sample. Growth then rotates its
`overlay.getRandomPeers` announcements across the whole known set. The scout
(45 s wait) reached `registered_peers=51`, `unique_pingers=2`,
`address_storing_peers=91`, and `pings_received=2` without a delivery, which
is a red window by the push-lottery model; the delivery gate remains the
measurement for both the sweep and the admission funnel. Both are tracked in
`TODO.md`.

## Full-node queries seen on the overlay

Peers address `tonNode.*` queries over the same ADNL session as overlay
traffic. One of them is implemented by this crate's peer, and the upstream
answer is now pinned down:

- `tonNode.getCapabilities = tonNode.Capabilities` - constructor id
  `0xdee618f8`, the CRC32 of the definition line without the trailing
  semicolon, the same scheme as every other id in `ton_api.tl`.
- `tonNode.capabilities#f5bf60c0 version_major:int version_minor:int flags:# = tonNode.Capabilities`
- Upstream answers from `validator/full-node-queries.hpp:469`
  (`FullNodeQueries::process_query(ton_api::tonNode_getCapabilities, ...)`):
  `create_serialize_tl_object<ton_api::tonNode_capabilities>(FullNode::PROTO_VERSION_MAJOR, FullNode::PROTO_VERSION_MINOR, 0)`,
  with `PROTO_VERSION_MAJOR = 3` and `PROTO_VERSION_MINOR = 2` from
  `validator/full-node.h:186-187`, and `flags = 0`.
- Dispatch is `FullNodeQueries::handle_query` (`:209`), which fetches a
  `ton_api::Function` and `ton_api::downcast_call`s into the per-type
  `process_query` overloads; the generic template at `:237` returns
  `unknown query` for everything else. So the handler set is closed and a
  non-validator answering `tonNode.capabilities 3 2 0` is indistinguishable
  from a full node for this query.

Until this crate answers it, the query lands in `queries_unhandled` with
`last_unhandled_query_id = 0xdee618f8`, which is the observable to grep for
in a live run. `pongs_sent` is unaffected: `overlay.ping` and
`tonNode.getCapabilities` are separate dispatch paths.

### Observed on mainnet

In a 300 s live run against sixteen mainnet seeds, this node received 22
overlay queries in total: 5 `overlay.ping`, 4 `overlay.getRandomPeers`,
1 `overlay.getRandomPeersV2` and **12 `tonNode.getCapabilities`**. The
capabilities probe was therefore the single most common inbound query, and
the only substantial source of `queries_unhandled`.

It is now answered by `tonutils_tl::tl::fullnode::TonNodeCapabilities`, wired
through `OverlayQuery::GetCapabilities` in
`crates/tonutils-mempool/src/overlay_inbound.rs`, and counted as
`ProtocolStats::capabilities_answers`. `TonNodeGetCapabilities` carries the
explicit id `0xdee618f8` because the definition line has no `#` and the
checked-in schema snapshot records the tag as empty; the value is the one
observed on the wire, not a recomputed one.

`overlay.getRandomPeersV2` is answered as well: the V2 records
carry `flags` (zero) and an empty member certificate, and because
`flags == 0` upstream signs them with the same
`overlay.node.toSign` definition as V1
(`OverlayNode::to_sign` in `overlay/overlay-id.hpp`), so the
signature is the V1 signature byte for byte.  The types live in
`tonutils_tl::tl::overlay`, split out of `network` to keep both
files under the repository line limit, and are re-exported from
`tonutils_tl::tl::network` so every existing import path keeps
working.

Iterative overlay-node resolution, official-node packet fixtures, upstream
pending-set acceptance evidence for this node's `overlay.node` record, and
production mempool broadcast selection remain TODO items; the session trait
still lets applications supply those higher-level policies.
