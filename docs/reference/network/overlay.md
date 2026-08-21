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
- `tonutils-adnl` owns the opt-in UDP datagram codec; it does not silently turn
  UDP into a stream or implement QUIC.

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
project's WebSocket interface is not part of this crate. Canonical DHT/overlay
TL constructors and production peer bootstrap remain TODO items until protocol
evidence is checked in; the session trait is a transport boundary, not an ADNL
handshake implementation.
