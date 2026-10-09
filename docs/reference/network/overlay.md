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
  authenticated `AdnlUdpSession` primitives; it does not implement QUIC.

## Fast-path model

`OverlayPeerPool` rejects packets over `max_packet_size` before enqueueing,
requires a registered peer, and applies bounded backpressure through Tokio's
multi-producer channel. Consumers can keep a persistent receive loop separate
from application callbacks. `RoutingMetadata` records overlay, peer, receive
time, and hop count without decoding the payload.

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
