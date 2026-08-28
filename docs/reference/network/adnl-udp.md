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

`tonutils-adnl` exposes `AdnlUdpPeer`, `AdnlUdpSocket`, and authenticated
direct/channel packet primitives behind the opt-in `udp` feature. Direct
packets use the upstream layout of destination id, ephemeral Ed25519 key,
SHA-256 digest, and AES-CTR ciphertext. Established channel packets use the
channel id, canonical AES-channel digest/key/IV derivation, TL
`adnl.packetContents`, bounded sequence replay tracking, and ACK validation.
All datagram APIs enforce the 64 KiB bound and provide Tokio timeouts.

`AdnlUdpSession` expects the caller to provision the remote identity and now
supports create/confirm channel negotiation plus typed DHT and overlay query
helpers. Fragmentation and NAT traversal remain outside this layer.

## Missing Work

- Add packet fixtures from official nodes.
- Add deterministic simulated UDP tests.
