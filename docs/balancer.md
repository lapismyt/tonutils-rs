# LiteBalancer

`LiteBalancer` is the current multi-peer wrapper around connected
`LiteClient` instances. It is available with the `liteclient` feature and is
intended for callers that want the same LiteAPI helper surface with basic peer
selection and retry behavior.

Audience: applications that can connect to several liteservers and want basic
failover. Prerequisites: `liteclient` feature, live network access, and explicit
peer construction from config or socket/public-key pairs. This is not a
consensus or proof-verification layer.

## Peer Setup

The balancer owns already constructed clients. When using public global config,
create one `LiteClient` per selected liteserver, then call `start_up` before
sending requests.

```rust
use std::str::FromStr;
use std::time::Duration;
use tonutils::liteclient::{
    balancer::LiteBalancer,
    client::LiteClient,
    rate_limit::RequestRateLimit,
};
use tonutils::network_config::ConfigGlobal;

async fn example(config_json: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut peers = Vec::new();

    for index in 0..config.liteservers.len().min(3) {
        peers.push(LiteClient::connect_config(&config, index).await?);
    }

    let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10))
        .with_rate_limit_per_peer(RequestRateLimit::per_second(5)?)
        .with_global_rate_limit(RequestRateLimit::per_second(12)?);
    balancer.start_up().await?;
    let info = balancer.get_masterchain_info().await?;
    println!("{}", info.last.seqno);
    balancer.close_all().await?;
    Ok(())
}
```

`start_up` marks connected peers as alive, performs a best-effort archival
probe, starts the current health-check task, and records the balancer as
initialized. `close_all` aborts the health-check task and closes all owned
clients.

## Request Routing

The balancer exposes typed helpers for common LiteAPI calls, including
masterchain info, version, time, block and state loading, block headers,
account state, get-methods, transactions, shard info, config, libraries, and
message sending. Pytoniq-like typed methods such as `raw_get_block`,
`get_account_state_simple`, `run_get_method_typed`, and
`get_config_params_typed` delegate to the underlying `LiteClient` through the
same peer selection and retry path. Request routing builds a priority list from
alive peers, observed masterchain seqno, average response time, and current
in-flight request count.

For non-archival calls, no peer quorum is established. The first successful
peer response is returned, and failed peers are marked dead for later requests.
For calls that need archival data, the balancer uses peers detected by its
archival probe.

## Request Rate Limits

Per-peer limits throttle each owned `LiteClient`. Global limits throttle total
balancer attempts, including retries and each `send_message` peer attempt.
Neither limit is enabled by default.

Use per-peer limits for rented liteserver quotas that apply separately to every
server. Use a global limit when an upstream account or proxy enforces an
aggregate request budget.

## Current Limits

This is a prototype balancer, not a production peer manager yet:

- peer transitions are represented, but timeout and reconnect state machines
  are still incomplete;
- reconnection uses no stored peer descriptors, exponential backoff, or jitter;
- latency scoring uses an arithmetic average rather than EWMA;
- stale seqno and in-flight penalties are basic;
- `send_message` failover does not yet preserve every peer error for detailed
  diagnostics;
- proof fields returned by LiteAPI calls are not verified.

Use it as a convenience layer over trusted liteserver connections. Do not treat
multi-peer routing as proof verification or consensus validation.
