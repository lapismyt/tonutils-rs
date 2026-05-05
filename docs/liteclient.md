# LiteClient

`LiteClient` sends LiteAPI requests directly to TON liteservers over native
ADNL TCP. It is available with the `liteclient` feature, which also enables TL,
TVM, and ADNL TCP support. It supports typed request helpers and raw LiteAPI
bytes for methods that are not yet wrapped by a convenience method.

## Connect From Global Config

```rust
use std::str::FromStr;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;

async fn example(config_json: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let info = client.get_masterchain_info().await?;
    println!("{}", info.last.seqno);
    Ok(())
}
```

## Raw Query

Use `query_raw` when a LiteAPI constructor is known by schema but does not yet
have a typed Rust convenience method. The input must be an already serialized
LiteAPI request body. The output is the raw serialized LiteAPI response body.

```rust
use std::str::FromStr;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
async fn example(config_json: &str, request: Vec<u8>) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let response = client.query_raw(request).await?;
    println!("{}", hex::encode(response));
    Ok(())
}
```

## Request Rate Limits

`LiteClient` is unlimited by default. To stay within a provider quota, attach a
local token-bucket limiter before sending requests:

```rust
use tonutils::liteclient::{
    client::LiteClient,
    rate_limit::RequestRateLimit,
};

async fn example(mut client: LiteClient) -> anyhow::Result<()> {
    client.set_rate_limit(RequestRateLimit::per_second(5)?);
    let info = client.get_masterchain_info().await?;
    println!("{}", info.last.seqno);
    Ok(())
}
```

The limiter waits asynchronously instead of failing fast. Typed helpers and raw
queries share the same `query_raw` path, so one configured limit covers both.

## Contract Helpers

`tonutils::contracts::Contract` reuses `LiteClient::get_masterchain_info`,
`LiteClient::get_account_state`, and `LiteClient::run_get_method`. It does not
change the LiteAPI trust model: proof fields are preserved, but proof
verification is not implemented yet.

## Typed Phase 1 Additions

Recent typed helpers added to `LiteClient`:

- `lookup_block_with_proof`
- `list_block_transactions_ext`
- `get_libraries_with_proof`
- `get_shard_block_proof`
- `get_out_msg_queue_sizes`
- `get_block_out_msg_queue_size`
- `get_dispatch_queue_info`
- `get_dispatch_queue_messages`
- `get_nonfinal_validator_groups`
- `get_nonfinal_candidate`
- `get_nonfinal_pending_shard_blocks`

Example:

```rust
use tonutils::liteclient::client::LiteClient;
use tonutils::tl::{BlockId, BlockIdExt};

async fn example(client: &mut LiteClient, block_id: BlockId, mc_block_id: BlockIdExt) -> anyhow::Result<()> {
    let proof = client
        .lookup_block_with_proof((), block_id, mc_block_id, None, None)
        .await?;
    println!("{}", proof.id.seqno);
    Ok(())
}
```

## Current Limits

The client has typed helpers for the common LiteAPI surface and a raw byte
escape hatch for missing constructors, including typed nonfinal pending-shard
block calls via `get_nonfinal_pending_shard_blocks`. Proof verification is not implemented.
Timeout configuration is currently limited, and live-network behavior depends
on the selected liteserver.

## Nonfinal Typed Calls

Nonfinal constructors expose candidate and validator-group data that may change
before finalization. They are useful for diagnostics and research flows and do
not include proof verification or production-safety guarantees.

```rust
use tonutils::liteclient::client::LiteClient;
use tonutils::tl::NonfinalCandidateId;

async fn example(client: &mut LiteClient, candidate_id: NonfinalCandidateId) -> anyhow::Result<()> {
    let groups = client.get_nonfinal_validator_groups(None).await?;
    println!("{}", groups.groups.len());

    let candidate = client.get_nonfinal_candidate(candidate_id).await?;
    println!("{}", candidate.data.len());
    Ok(())
}
```

## Shell Equivalent

```bash
tonutils --output json liteclient masterchain-info --ls-index 0
tonutils --rps 5 --output json liteclient masterchain-info --ls-index 0
tonutils --output hex liteclient raw-query --ls-index 0 --hex '<serialized-request-hex>'
tonutils --output json contract run-get-method --address '<addr>' --method seqno
```
