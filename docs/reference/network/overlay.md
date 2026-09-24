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

## Reference and unfinished work

The pending-message behavior is compared conceptually with
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool). The Python
project's WebSocket interface is not part of this crate. Canonical ADNL/DHT/
overlay TL constructors, direct UDP live probing, channel create/confirm state
transitions, signed overlay join queries, and transport-to-stream delivery are
covered by checked fixtures and localhost tests. Iterative overlay-node
resolution, official-node packet fixtures, and production mempool broadcast
selection remain TODO items; the session trait still lets applications supply
those higher-level policies.
