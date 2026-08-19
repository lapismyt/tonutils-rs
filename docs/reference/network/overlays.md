# TON Overlays

Overlays are logical peer groups over ADNL. They are used by validators and full nodes for shardchain communication, broadcasts, and peer exchange.

## Why Overlays Matter

LiteAPI is request/response. Mempool and pending-message observation requires more direct network participation, which usually means DHT discovery and overlays.

## Concepts

- overlay id,
- overlay node,
- overlay peer list,
- random peer exchange,
- broadcast,
- certificates,
- validator group overlays,
- shard overlays.

## Implementation Requirements

An overlay implementation needs:

- ADNL UDP transport,
- DHT peer discovery,
- overlay TL types,
- peer score and expiration,
- query routing,
- broadcast validation,
- deduplication.

## Mempool Relation

Pending external messages are propagated before final block inclusion. A scanner must distinguish:

- observed broadcast,
- candidate block data,
- included transaction,
- finalized transaction.

## Missing Work

- Extract overlay constructors from `ton_api.tl`.
- Document overlay id derivation.
- Study validator shard overlays relevant to pending messages.
- Add captured fixtures.
