# Mempool Scanning Research

TON pending-message observation is not the same as reading finalized transactions. A scanner must expose uncertainty explicitly.

## Data Sources

Potential sources:

- overlay broadcasts,
- nonfinal LiteAPI methods,
- dispatch queue LiteAPI methods,
- candidate block data,
- finalized block transaction lists.

## Protocol Dependencies

Likely required:

- ADNL UDP,
- DHT discovery,
- overlay peer exchange,
- broadcast decoding,
- message BoC decoding,
- duplicate detection.

## API Shape

Future scanner API should expose:

- async stream of pending items,
- message hash,
- raw BoC,
- first seen timestamp,
- source peer,
- shard hint if known,
- confidence or stage enum.

Suggested stages:

- `ObservedBroadcast`,
- `CandidateIncluded`,
- `FinalizedIncluded`,
- `DroppedOrExpired`.

## Safety Notes

Pending data can disappear. Do not expose pending observations as confirmed transactions. Users must opt into mempool semantics.

## Research Tasks

- Study `yungwine/ton-mempool`.
- Identify overlay ids used for relevant traffic.
- Capture sample packets.
- Compare with nonfinal LiteAPI responses.
