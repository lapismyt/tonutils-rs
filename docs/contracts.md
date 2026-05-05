# Contracts

The contract API is available with the `liteclient` feature. It is a thin
wrapper over LiteAPI account-state and get-method requests, so it works with
both `LiteClient` and `LiteBalancer`.

## Account State

```rust
use std::str::FromStr;
use tonutils::contracts::Contract;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::Address;

async fn example(config_json: &str, address: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let address = Address::from_str(address)?;
    let mut contract = Contract::new(&mut client, address);
    let state = contract.get_state_latest().await?;
    println!("{}", state.state.len());
    Ok(())
}
```

`get_state_latest` first reads `getMasterchainInfo` and then calls
`getAccountState` for the returned masterchain block. The response preserves the
raw account-state BoC and proof bytes.

## Get-Methods

```rust
use std::str::FromStr;
use tonutils::contracts::{Contract, RunMethodResultExt};
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::{Address, TvmStack};

async fn example(config_json: &str, address: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let address = Address::from_str(address)?;
    let mut contract = Contract::new(&mut client, address);
    let result = contract
        .run_get_method_by_name_latest("seqno", TvmStack::empty())
        .await?;
    let stack = result.decode_result_stack()?;
    println!("exit_code={} decoded={}", result.exit_code, stack.is_some());
    Ok(())
}
```

Method names use the standard TON mapping:
`(crc16(method_name) & 0xffff) | 0x10000`. Numeric ids can be passed directly
with `run_get_method` or `run_get_method_latest`.

## Result Decoding

`RunMethodResultExt` exposes:

- `raw_result_boc()`: borrowed raw `result` BoC bytes.
- `decode_result_stack()`: attempts to decode supported stack values.
- `result_stack_lossless()`: returns decoded stack values or preserves the raw
  undecodable bytes with the decode error.

The current stack codec supports nulls, integers, cells, slices, tuples, lists,
and explicit unsupported payloads in this crate's internal representation. Stack
compatibility with all liteserver return shapes is still being expanded. Get
method inputs currently use typed `TvmStack` values; the CLI only exposes
empty-stack get-method calls.

## Proofs

LiteAPI proof fields are returned as raw bytes. This crate does not verify
account-state, shard, or get-method proofs yet, so callers must not treat these
helpers as a proof-verifying light client API.
