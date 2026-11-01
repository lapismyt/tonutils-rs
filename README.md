# tonutils

`tonutils` is a pure-Rust SDK for The Open Network (TON). It implements TON
cells, Bag of Cells (BoC), TL-B structures, TL/LiteAPI wire types, ADNL,
overlay transport, wallets, and a direct liteserver client without depending on
a third-party TON SDK or a runtime native library.

Features are opt-in and the default feature set is empty. Enable only the
layers your application uses.

## Coming From EVM Or Solana

TON stores state and messages in a directed acyclic graph of **cells**. A cell
contains up to 1,023 data bits and four references. A **Bag of Cells** is the
serialized container used to move a cell graph over the wire and into
transactions. **TL-B** gives typed meaning to cell layouts.

| Familiar concept | TON and `tonutils` |
| --- | --- |
| EVM state trie or Solana account bytes | Cell DAG, BoC encoding, and TL-B models in `tvm` and `tlb` |
| JSON-RPC provider | Binary LiteAPI over ADNL through `liteclient` |
| ABI-encoded calldata | Cell payloads and TL-B message bodies |
| Local transaction construction and signing | Offline wallet and contract helpers |
| Pending-transaction feed | `MempoolServer` over an application-built `overlay::OverlayClient` |

The layers compose, but none of them is required to use the others. In
particular, cell construction, wallet address derivation, signing, and message
encoding work without a network connection.

## Quickstart

For offline cells, TL-B structures, and wallet construction:

```toml
[dependencies]
tonutils = { version = "2", features = ["tvm", "tlb", "wallet"] }
```

For a direct liteserver client, add the network layers you need:

```toml
[dependencies]
tonutils = { version = "2", features = ["liteclient"] }
```

### Offline Cell Example

```rust
use tonutils::tvm::{Builder, deserialize_boc, serialize_boc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = Builder::new();
    builder.store_u32(0xdead_beef)?;

    let root = builder.build()?;
    let boc = serialize_boc(&root, true)?;
    let decoded = deserialize_boc(&boc)?;

    assert_eq!(root.hash(), decoded.hash());
    println!("encoded {} bytes", boc.len());
    Ok(())
}
```

### Live Read Shape

```rust
use tonutils::{liteclient::LiteClient, network_config::ConfigGlobal};

async fn latest_masterchain_seqno(
    config_json: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let config: ConfigGlobal = config_json.parse()?;
    let mut client = LiteClient::connect_first(&config).await?;
    let info = client.get_masterchain_info().await?;
    Ok(info.last.seqno)
}
```

The live example requires a Tokio runtime and application-selected configuration.
It is intentionally not a proof that a liteserver answered honestly.

## Features

| Feature | Provides | Requires live network |
| --- | --- | --- |
| `tvm` | Cells, builders, slices, BoC, addresses, dictionaries, TVM stacks, and `TvmError` | No |
| `tl` | Shared TL primitives, TL schema snapshots, generated metadata, and schema helpers | No |
| `tlb` | Typed TL-B blocks, messages, transactions, accounts, and proof structures | No |
| `wallet` | TON mnemonic handling, Ed25519 signing, Wallet V4R2/V5R1 addresses, and external message BoCs | No |
| `crc` | CRC16 and CRC32 helpers used by TON identifiers and BoC formats | No |
| `metadata` | TEP-64 on-chain and off-chain content parsing | No |
| `network-config` | Global network configuration parsing and liteserver selection | No |
| `adnl` | ADNL identity, encrypted UDP transport, QUIC transport, and ADNL TL wire types | Yes for transport |
| `liteclient` | LiteAPI types, direct client, balancer, and typed BoC decode helpers | Yes |
| `contracts` | Provider traits, account-state reads, and get-method stack helpers | Yes for reads |
| `overlay` | ADNL UDP/QUIC overlay sessions, discovery, peer management and persistence, inbound serving, Plumtree public-overlay broadcasts, legacy signed simple/FEC broadcasts, DHT publication, STUN, and protocol statistics | Yes for transport |
| `overlay-config` | Global-config bootstrap download and parsing for `OverlayNodeBuilder`; implies `overlay` and `network-config` | Yes for download |
| `mempool` | External-message validation, deduplication, events, and client/server submission through a caller-supplied overlay client | Yes, through `overlay` |
| `serde` | Serde support for enabled public types that expose it, currently ADNL public keys | No |

`tl` owns the snapshots and shared primitives, not every TON protocol schema.
Handwritten ADNL, LiteAPI, and overlay wire types live in `adnl::tl`,
`liteclient::tl`, and `overlay::tl` respectively. The ownership map and wire
invariants are documented in [docs/reference/README.md](docs/reference/README.md).

## Overlay And Mempool

`overlay` owns every transport and network-maintenance concern: ADNL UDP and
QUIC sessions, factories and lookups, bootstrap and discovery, peer growth,
peer quality and persistence, inbound query answering, membership gossip,
capabilities, DHT address publication, STUN, packet pre-deduplication, and
signed simple/FEC broadcasts. Build a client with
`OverlayNodeBuilder::start_client()` or a serving node with
`start_server()`.

Public-overlay payloads use Plumtree by default. Configure it with
`OverlayNodeBuilder::plumtree`, then call
`OverlayClient::broadcast_plumtree_simple` or
`OverlayClient::broadcast_plumtree_fec`; the server variants expose the same
operations. The returned count is the number of payload frames immediately
queued for eager neighbours, not a delivery guarantee. Use
`plumtree_stats()` for local receive, send, completed-broadcast, repair, and
validation counters.

`OverlayClient::broadcast_signed` is the legacy single-object signed broadcast
path. It remains the wire path for external-message traffic, including
`MempoolClient::send_external`, because that payload is a
`tonNode.externalMessageBroadcast` rather than a generic Plumtree payload. It
does not use Plumtree eager feedback or repair; new public-overlay bulk traffic
should use the Plumtree methods.

`start_client` does not publish an ADNL address, so other members cannot select
the node as a relay. `start_server` enables DHT publication and relay
participation, but reachability, overlay membership rules, and peer selection
still determine whether mainnet peers route a particular broadcast through the
node. Neither mode guarantees application delivery.

`mempool` owns only external-message semantics. `MempoolServer::new` takes an
already-built `OverlayClient` and a `MempoolConfig`, subscribes to verified
overlay packets and peer statuses, validates and deduplicates external-message
BoCs, and emits `MempoolEvent` values. The returned `MempoolClient` submits
external messages through that same overlay client; it never starts transport,
performs discovery, or publishes DHT records. Enable `overlay-config` only when
the application wants the builder to download global-config bootstrap data.

`OverlayError` owns bootstrap, discovery, session, peer-store, broadcast, and
overlay-configuration failures. `MempoolError` is limited to external-message
validation/decoding, submission, event-queue, and mempool-configuration
failures.

## Offline Versus Live

Offline work is deterministic: construct cells, serialize or parse BoCs, derive
wallet addresses, sign messages, and encode TL-B/TL payloads. This path needs no
credentials and makes no network trust assumptions.

Live work starts when the application connects through ADNL or LiteAPI, loads a
global network configuration, joins an overlay, or submits a message. In that
mode the application chooses the liteserver, network, public keys, timeouts,
retry policy, and proof-validation policy. A successful LiteAPI response or
message submission is not, by itself, finality or inclusion proof. Decode
helpers preserve evidence, but callers must opt in to the proof checks that
their threat model requires.

### Live Network Gates

Live checks are ignored by default and require a downloaded public global
config. Run each check through the exact-test wrapper so a renamed or removed
test cannot produce a green zero-test result:

```bash
export TON_GLOBAL_CONFIG_JSON="$(curl -fsSL https://ton.org/global.config.json)"
export TON_TESTNET_GLOBAL_CONFIG_JSON="$(curl -fsSL https://ton.org/testnet-global.config.json)"

scripts/run-live-test.sh live_probe mainnet_dht_find_node_returns_valid_nodes
scripts/run-live-test.sh live_probe testnet_dht_find_node_returns_valid_nodes
scripts/run-live-test.sh live_probe mainnet_udp_overlay_get_random_peers_returns_valid_nodes
scripts/run-live-test.sh live_probe mainnet_quic_overlay_get_random_peers_returns_valid_nodes
scripts/run-live-test.sh live_delivery mainnet_public_plumtree_receives_payload
```

The DHT gate requires a nonempty `dht.findNode` answer whose nodes all validate.
The UDP and QUIC gates require discovery plus a valid nonempty
`overlay.getRandomPeers` response. The delivery gate starts an
`OverlayNodeBuilder` server and `MempoolServer`, then requires both a completed
Plumtree broadcast and an application packet from `subscribe_packets()` within
one of three 120-second attempts. There is no network-unavailable skip:
unreachable public infrastructure or missing payload traffic keeps the job red.
Each successful log records target, peer, request, elapsed time, and node or
payload counts. CI uploads the raw per-test logs as artifacts.

## Workspace

- `tonutils` is the library.
- `tonutils-cli` provides the `tonutils` command-line binary.
- `tonutils-macros` provides TL-B derive macros.
- `tonutils-schema-gen` validates checked-in schema metadata and inventory.

Use the crate's rustdoc for the public API. Repository-level development and
verification instructions are in [CONTRIBUTING.md](CONTRIBUTING.md). The crate
is licensed under MIT.
