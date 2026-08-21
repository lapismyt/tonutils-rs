# ADNL UDP

ADNL UDP is required for general TON peer-to-peer networking, DHT, overlays,
and mempool scanning. It is not the same implementation path as ADNL TCP
liteserver connections.

## Expected Responsibilities

UDP ADNL must handle:

- datagram boundaries,
- peer address lists,
- packet contents flags,
- public key identity,
- signatures,
- channel creation and confirmation,
- reinit dates,
- sequence numbers,
- packet parts for large messages.

## Relevant TL Areas

`ton_api.tl` contains ADNL packet and message definitions such as:

- `adnl.packetContents`,
- `adnl.message.createChannel`,
- `adnl.message.confirmChannel`,
- `adnl.message.custom`,
- `adnl.message.query`,
- `adnl.message.answer`,
- `adnl.message.part`,
- `adnl.addressList`,
- `adnl.node`.

## Implementation Risks

- UDP packet loss and reordering.
- Large message fragmentation.
- NAT and address list freshness.
- Correct signature coverage.
- Interaction with DHT and overlay routing.

## Crate Design

`tonutils-adnl` exposes `AdnlUdpPeer` and `AdnlUdpSocket` behind the opt-in
`udp` feature. They keep one encrypted ADNL frame per datagram, enforce a
64 KiB datagram bound, reject trailing bytes and replayed ciphertexts in a
bounded window, and provide Tokio send/receive operations with timeouts. The
helpers deliberately do not claim to implement channel negotiation,
fragmentation, or NAT traversal; those concerns belong to the session and
protocol layers above them.

## Missing Work

- Implement and fixture-test channel negotiation and packet-content TL.
- Add packet fixtures from official nodes.
- Add deterministic simulated UDP tests.
