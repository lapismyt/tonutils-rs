# ADNL UDP

ADNL UDP is required for general TON peer-to-peer networking, DHT, overlays, and future mempool scanning. It is not the same implementation path as ADNL TCP liteserver connections.

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

The future UDP implementation should not reuse TCP framing. It should share only crypto identity types and TL message types where protocol-compatible.

## Missing Work

- Parse and document all relevant ADNL UDP TL constructors.
- Add packet fixtures from official nodes.
- Add deterministic simulated UDP tests.
