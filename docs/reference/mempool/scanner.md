# Mempool scanner design

`tonutils-mempool` reports what an overlay receive path has seen. It does not
claim that a message is accepted by a validator and it does not replace
LiteServer block/indexing queries.

## Events and lifetime

- `ExternalMessage` means **Seen**: the envelope passed the configured fast
  checks and its SHA-256 hash was not observed in the scanner lifetime.
- `Included` is a separately supplied correlation result from a slow path.
- `PeerStatus` reports transport lifecycle only.
- A message that was deduplicated is not emitted again; an unknown final state
  remains **Unknown**, not `Included`.

The raw BoC is held in `Arc<[u8]>`, so event consumers and broadcast peers can
share ownership without copying the payload. The bounded event queue provides
backpressure. Deduplication is sharded by the first hash byte; eviction is not
yet implemented, so deployments must bound scanner lifetime or add a future
TTL/size policy.

## Fast and slow paths

The fast path checks size, minimum envelope length, and (by default) the BoC
magic `b5ee9c72`, hashes the raw bytes, inserts the hash into a shard, and
publishes immediately. Destination and body decoding are intentionally absent
from this path. A future slow worker can decode TL-B, persist observations,
query LiteServer, and call `mark_included`.

## Reference comparison

The architecture is informed by
[`yungwine/ton-mempool`](https://github.com/yungwine/ton-mempool): receive from
multiple peers, deduplicate, broadcast, and correlate inclusion. The Rust API
uses `Stream` and does not expose a WebSocket compatibility layer.

## Current gaps

Live overlay bootstrap, canonical external-message TL/TL-B extraction,
destination decoding, TTL-based dedup eviction, and LiteServer inclusion
tracking need upstream schemas, fixtures, and live-network tests before being
described as complete.
