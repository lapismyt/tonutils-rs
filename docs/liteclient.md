# LiteClient

`LiteClient` sends LiteAPI requests directly to TON liteservers over native
ADNL TCP. It is available with the `liteclient` feature, which also enables TL,
TVM, and ADNL TCP support. It supports typed request helpers and raw LiteAPI
bytes for methods that are not yet wrapped by a convenience method.

Audience: callers that already have a liteserver endpoint or TON global config.
Prerequisites: async Rust runtime, `liteclient` feature, and live network access.
For multi-peer retry behavior, see [LiteBalancer](balancer.md). For shell
commands over the same APIs, see [CLI](cli.md).

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

## Decoded BoC Helpers

`tonutils::liteclient::boc` contains offline decode helpers for LiteClient
payloads. Each helper preserves the raw BoC bytes and decoded root cell, then
adds a typed view where Phase 1 models exist:

- `decode_account_state_boc` -> `Account`
- `decode_block_boc` -> generated-backed `Block` wrapper
- `decode_config_params_boc` -> `ConfigParams`
- `decode_shard_state_boc` -> `ShardState`
- `decode_merkle_proof_boc` and `decode_merkle_update_boc` -> exotic proof
  primitive wrappers

These helpers intentionally do not verify liteserver proofs by default. The
Merkle wrappers expose `verify_virtual_hash` and `verify_virtual_hashes` for
the local exotic-cell child-hash invariant only; callers must still anchor
proofs to trusted block ids before using decoded data as trusted state.

```rust
use tonutils::liteclient::boc::decode_account_state_boc;

fn example(raw_state_boc: &[u8]) -> anyhow::Result<()> {
    let decoded = decode_account_state_boc(raw_state_boc)?;
    println!("{}", decoded.boc.root_hash_hex());
    println!("{:?}", decoded.account);
    Ok(())
}
```

## Typed Phase 1 Additions

Recent typed helpers added to `LiteClient`:

- `raw_get_block` and `raw_get_block_data`
- `raw_get_block_header`
- `get_account_state_typed`, `raw_get_account_state`, and
  `get_account_state_simple`
- `raw_get_shard_info` and `raw_get_all_shards_info`
- `get_one_transaction_typed`, `raw_get_transactions`, and
  `raw_get_block_transactions_ext`
- `run_get_method_typed`
- `get_config_all_typed` and `get_config_params_typed`
- `get_libraries_typed` and `get_libraries_with_proof_typed`
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
escape hatch for missing constructors, including typed BoC decode wrappers for
block, account, transaction, shard, config, library, and get-method result
payloads. Full trust-level proof verification and full `block.tlb` expansion
are not implemented. Timeout configuration is currently limited, and
live-network behavior depends on the selected liteserver.

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
