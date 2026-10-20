# TON Mempool Scanner in Rust: Streaming Pending External Messages

`MempoolScanner` joins the TON mempool overlay over native ADNL/UDP or QUIC,
discovers overlay members through the DHT, and streams every external message
a member broadcasts to this node as an async event. It re-broadcasts what it
accepts to its connected members, so a scanner both reads and relays the
pending-message stream.

Audience: applications that consume the live mempool - monitoring, spam
screening, MEV/misbehavior research, or message relays. Prerequisites: a
tokio runtime, outbound UDP (or QUIC) reachability to the mainnet seeds from
the global config, and no lite servers or API keys. Live network access is
required for everything in this guide; the crate itself has no feature flags
- it enables `tonutils-adnl`'s `udp` and `quic` features unconditionally.

## Building a Scanner

```rust
use std::net::SocketAddr;
use std::time::Duration;

use tonutils_adnl::KeyPair;
use tonutils_mempool::MempoolScannerBuilder;

# async fn build() -> Result<(), Box<dyn std::error::Error>> {
let (scanner, manager, events) = MempoolScannerBuilder::new()
    // Fetch ton.org's global config; pass `global_config_json` instead
    // for a local file or an air-gapped setup.
    .download_config(true)
    .discovery_timeout(Duration::from_secs(30))
    // Derives the mainnet basechain overlay id from the zero state
    // hash and wires DHT discovery for it in one call.
    .native_udp_for_shard_public(
        "0.0.0.0:0".parse()?,
        KeyPair::generate(&mut rand::rngs::OsRng),
        0,
        i64::MIN,
        [
            0x5e, 0x99, 0x4f, 0xcf, 0x4d, 0x42, 0x5c, 0x0a, 0x6c, 0xe6, 0xa7, 0x92, 0x59, 0x4b,
            0x71, 0x73, 0x20, 0x5f, 0x74, 0x0a, 0x39, 0xcd, 0x56, 0xf5, 0x37, 0xde, 0xfd, 0x28,
            0xb4, 0x8a, 0x0f, 0x6e,
        ],
        None,
    )
    .start()
    .await?;
# Ok(())
# }
```

`start()` returns the scanner (shared handle), the peer manager, and the
event stream. The manager owns the connection set; call
`manager.shutdown_wait()` for a clean exit.

Transport choices:

- `native_udp_for_shard_public(...)` - the mainnet basechain overlay over
  ADNL/UDP (shown above);
- `native_udp(addr, key, channel_timeout)` with `overlay_id(...)` or
  `dht_overlay_key(...)` - any explicit overlay;
- `native_quic(addr, key)` - QUIC sessions instead of ADNL/UDP;
- `native_udp_seeds_only(addr, key, timeout)` - sessions to explicit
  `.seeds(...)` only, no DHT discovery; for private or test setups.

Bootstrap seeds (explicit `.seed(...)`/`.seeds(...)` and the global
config's `dht.static_nodes`) are DHT contacts: they answer `dht.findValue`
and `dht.store` and carry the discovery lookup, but they do not host the
mempool overlay, so membership comes from discovery once discovery returns
members. Use `pinned_peers(...)` for members that must stay in the
connection set regardless of discovery.

## Streaming Events

```rust
use futures::StreamExt;
use tonutils_mempool::MempoolEvent;

# async fn stream(mut events: impl futures::Stream<Item = MempoolEvent>) {
while let Some(event) = events.next().await {
    match event {
        // An accepted pending external message.
        MempoolEvent::ExternalMessage { hash, raw_boc, routing, .. } => {
            println!("{hash:?} from {:?} ({} bytes)", routing.peer, raw_boc.len());
        }
        // Only emitted for `scanner.mark_included(...)` - the scanner
        // does not watch the blockchain itself.
        MempoolEvent::Included { hash, .. } => println!("{hash:?} included"),
        MempoolEvent::PeerStatus(status) => println!("peer: {status:?}"),
    }
}
# }
```

Every accepted message is a complete external-message BoC (`raw_boc`), so
callers can parse it with `tonutils-tlb`, hash-check it, or hand it to their
own validation. A runnable version of this loop is
`crates/tonutils-mempool/examples/mempool_stream.rs`:

```bash
cargo run -p tonutils-mempool --example mempool_stream
```

## Validation, Dedup, and Sending

Inbound messages pass BoC and TL-B validation and a per-hash dedup window
before they are emitted; the knobs live in `MempoolConfig`:

```rust
use std::time::Duration;
use tonutils_mempool::{MempoolConfig, MempoolScannerBuilder};

# fn config() -> MempoolConfig {
MempoolConfig {
    max_message_size: 1 << 20,
    require_boc_magic: true,
    validate_message: true,
    dedup_ttl: Duration::from_secs(300),
    ..MempoolConfig::default()
}
# }
```

`scanner.send_external(raw_boc)` broadcasts this node's own external
message to the connected members - the same sampling-based fan-out upstream
uses, so it widens delivery but does not confirm it. `scanner.ingest(...)`
injects a message into the local stream without any network, which is how
offline tests and replays drive the same event path.

## Peer Quality and Churn

The scanner scores every connected member by delivery behavior
(`PeerQualityConfig`: 500 ms late threshold, 3 s miss window, 10
observations before a peer is judged, 60 s score half-life). Scores feed
`scanner.quality().snapshot()` (aggregate counts and score range) and, when
enabled, churn:

```rust
# use tonutils_mempool::MempoolScannerBuilder;
# fn builder() -> MempoolScannerBuilder {
MempoolScannerBuilder::new().quality_eviction(true)
# }
```

Quality eviction is **opt-in**: by default the connection set changes only
through discovery and growth. The churn task it enables evicts the worst
scores only when the managed roster exceeds `overlay_max_peers` **and** the
candidate carries at least `min_observations` delivery observations. With
this builder the roster exceeds the budget only through a lost peer's
reconnection re-registering after growth filled its slot (or a custom
session factory adding managed peers), and the default budget (30) sits
above typical live rosters, so evictions are rare by design.
`PeerQualityRegistry::evictions()` reports how many peers churn removed.

## Live Network Requirements

- Outbound UDP to the global config's seeds; a NAT'd host must be able to
  receive inbound UDP on its published port to be pushed broadcasts.
- Address publication runs STUN first; without an externally reachable
  address the node still reads the overlay but is harder for members to
  sample into their fan-out.
- Bootstrap discovery defaults to a 30 s lookup budget
  (`discovery_timeout`); the first member list arrives from that lookup.
- Delivery is sampling-based: whether a given message reaches this node
  depends on upstream fan-out (`get_neighbours` picks 5 propagators per
  hop). Absence of a message is not proof it does not exist. The recorded
  delivery arms live in
  `docs/reference/network/overlay-delivery-ledger.md`.

## Current Limits

- Delivery has no confirmation: an event proves a member pushed the
  message, silence proves nothing (see above).
- `MempoolEvent::Included` requires the caller to report inclusion via
  `mark_included`; the scanner does not follow blocks.
- Quality eviction is opt-in and judges only observed delivery traffic;
  thin evidence never evicts (`min_observations`), and without a roster
  over the `overlay_max_peers` budget it never evicts at all (see Peer
  Quality and Churn).
- The scanner does not verify TON proofs of anything; message validity is
  cell-level TL-B validation, not a consensus claim.
- QUIC transport (`native_quic`) works but the delivery gate and the
  recorded live evidence run on the ADNL/UDP path.

## Related Guides

- `docs/getting-started.md` - feature selection and dependency setup.
- `docs/networking.md` - ADNL transports and network config handling.
- `docs/examples.md` - the example catalog, including `mempool_stream`.
- `docs/reference/network/overlay-delivery-ledger.md` - measured delivery
  behavior of this implementation against mainnet.
