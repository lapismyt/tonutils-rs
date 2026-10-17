# TON Networking in Rust: ADNL, LiteAPI, and Global Config

The current networking surface includes native ADNL TCP for LiteAPI liteserver
connections, authenticated ADNL UDP sessions, typed DHT lookups, and a bounded
seed-only mempool overlay path.

Audience: callers configuring transport features and contributors separating
LiteAPI networking from UDP/DHT/overlay/mempool workflows.
Prerequisites: `adnl-tcp` for direct liteserver sockets, `network-config` for
global config parsing, and live network access for real liteserver calls.

## Feature Boundaries

- `adnl`: shared ADNL helper types and primitives.
- `adnl-tcp`: TCP transport, crypto handshake, frame codec, and peer wrapper.
- `liteclient`: LiteAPI client over ADNL TCP.
- `network-config`: TON global config JSON parsing and liteserver helpers.
- `tonutils-mempool`: seed-only overlay sessions, external-message validation,
  FEC reassembly, and bounded application delivery.
- `cli`: downloads public configs and exposes shell commands.

The default feature set enables `std`, `adnl-tcp`, and `liteclient`.
`network-config` and `cli` must be requested explicitly.

## ADNL TCP

`LiteClient::connect` accepts a socket address and liteserver public key. The
transport performs the native ADNL TCP handshake, then sends LiteAPI requests
through the framed encrypted stream.

```rust
use tonutils_liteclient::client::LiteClient;

async fn example(addr: &str, public_key: [u8; 32]) -> anyhow::Result<()> {
    let mut client = LiteClient::connect(addr, public_key).await?;
    let version = client.get_version().await?;
    println!("{}", version.version);
    Ok(())
}
```

Transport tests cover codec roundtrips, empty minimum-size payload frames,
client/server key and nonce directionality, partial frames, multi-frame
buffers, too-large payload rejection, tamper handling, and loopback handshake
behavior. Timeout configuration and graceful close APIs are still being
hardened.

## Network Config

`ConfigGlobal` parses TON global config JSON and exposes liteserver entries:

```rust
use std::str::FromStr;
use tonutils_network_config::ConfigGlobal;

fn example(config_json: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let first = config.first_liteserver()?;
    println!("{}", first.socket_addr());
    Ok(())
}
```

The config parser exposes liteservers and validated DHT static-node endpoints.
The mempool builder can use explicit `SeedPeer` values without any DHT
expansion, or opt into bounded typed DHT lookup.

## UDP, DHT, And Overlay

`AdnlUdpSession` handles authenticated direct packets, optional channel
create/confirm, sequence checks, and DHT/overlay query routing. The
`native_udp_seeds_only` scanner mode connects only configured peers and passes
validated `ExternalIn` messages through its `Stream` or callback handler.

These protocols remain separate from the ADNL TCP LiteAPI path. Iterative
overlay peer discovery and non-RaptorQ broadcast variants remain optional
follow-up work outside the minimal seed-only scanner path.
