# Networking

The current networking surface is native ADNL TCP for LiteAPI liteserver
connections plus optional public network config parsing. ADNL UDP, DHT,
overlays, and mempool networking are documented as future boundaries but are
not public runtime APIs yet.

Audience: callers configuring transport features and contributors separating
current LiteAPI networking from future DHT, overlay, and mempool work.
Prerequisites: `adnl-tcp` for direct liteserver sockets, `network-config` for
global config parsing, and live network access for real liteserver calls.

## Feature Boundaries

- `adnl`: shared ADNL helper types and primitives.
- `adnl-tcp`: TCP transport, crypto handshake, frame codec, and peer wrapper.
- `liteclient`: LiteAPI client over ADNL TCP.
- `network-config`: TON global config JSON parsing and liteserver helpers.
- `cli`: downloads public configs and exposes shell commands.

The default feature set enables `std`, `adnl-tcp`, and `liteclient`.
`network-config` and `cli` must be requested explicitly.

## ADNL TCP

`LiteClient::connect` accepts a socket address and liteserver public key. The
transport performs the native ADNL TCP handshake, then sends LiteAPI requests
through the framed encrypted stream.

```rust
use tonutils::liteclient::client::LiteClient;

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
use tonutils::network_config::ConfigGlobal;

fn example(config_json: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let first = config.first_liteserver()?;
    println!("{}", first.socket_addr());
    Ok(())
}
```

The config parser currently focuses on the `liteservers` section and Ed25519
public keys. It does not resolve DHT entries or overlay peers.

## Future Protocols

ADNL UDP will be the lower-level datagram transport needed by DHT and overlays.
DHT will resolve nodes and liteservers with signed peer records. Overlays will
carry overlay queries and broadcasts, including future mempool workflows.

These protocols are intentionally separate from the current ADNL TCP LiteAPI
path. Until they land, this crate cannot discover peers through DHT, join
overlays, or stream pending external messages from the mempool.
