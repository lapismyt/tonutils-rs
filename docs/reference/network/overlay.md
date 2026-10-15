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
  inserts into `neighbours_` only while `neighbours_.size() < max_neighbours()`
  (10); otherwise the node sits in `peers_` and waits for
  `update_neighbours`.
- **`update_neighbours(0)` runs every second** and fills free neighbour slots
  from random live members, while `update_neighbours(2)` - scheduled every
  30-120 s - replaces two random slots. So a freshly verified member is picked
  up quickly when a slot is free, and otherwise depends on the member set size
  for a lottery ticket roughly every 75 s.
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

The practical consequence is a bimodal delivery distribution. Admission by
the few full nodes in the overlay works through the query path; a bot learns
this node only when a full node samples it into a `getRandomPeers` answer
(`4 / |peers_|` per answer) and the bot then resolves its DHT `address` value
(`DhtClient.get_overlay_node`). One such listing typically cascades through
the small bot sets within minutes, which matches the observed 129-192 s
delivery runs - and its absence in a 300-600 s window proves nothing about
this node's reachability.

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
verified peer. Post-bootstrap growth is also currently inert on that workload:
`bootstrap` registers 32 sessions against `overlay_max_peers` of 30, so the
growth loop stops after its first ticks, and its one-shot session to an
already-connected member loses that member's stored-address race to the member's
own live session and times out instead of returning candidates. Both are
tracked in `TODO.md`.

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
