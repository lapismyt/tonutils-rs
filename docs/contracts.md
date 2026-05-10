# Contracts

The contract API is available with the `liteclient` feature. It is a thin
wrapper over LiteAPI account-state and get-method requests, so it works with
both `LiteClient` and `LiteBalancer`. The optional `contract-derive` feature
adds `#[derive(Contract)]` for contracts defined by fixed code BoC bytes and
typed TL-B data.

Audience: callers that need derived contract addresses, account state,
get-method execution, or a typed wrapper before wallet and ABI builders land.
Prerequisites: `liteclient`, TVM stack familiarity for non-empty get-method
arguments, and live network access. Current helpers preserve proof bytes but do
not verify them.

## Contract Blueprints

Use `ContractBlueprint` when a contract address is determined by code plus
typed data. With `contract-derive`, the root `tonutils::Contract` derive
implements the blueprint trait for a struct with exactly one named `data`
field.

```rust
use tonutils::Contract;
use tonutils::contracts::ContractBlueprint;
use tonutils::tlb::{Tlb, TlbSerialize};
use tonutils::tvm::TvmStack;

const WALLET_CODE_BOC: &[u8] = include_bytes!("wallet_v4r2.code.boc");

#[derive(Debug, Clone, Tlb)]
struct WalletData {
    seqno: u32,
    subwallet_id: u32,
    public_key: [u8; 32],
}

#[derive(Debug, Clone, Contract)]
#[contract(code = WALLET_CODE_BOC, workchain = 0)]
struct WalletV4R2 {
    data: WalletData,
}

async fn example<P: tonutils::contracts::ContractProvider>(
    client: &mut P,
    data: WalletData,
) -> anyhow::Result<()> {
    let wallet = WalletV4R2 { data };
    let address = wallet.address()?;
    let mut contract = wallet.bind(client)?;
    let seqno = contract
        .run_get_method_by_name_typed_latest("seqno", TvmStack::empty())
        .await?;
    println!("{} {}", address.to_raw(), seqno.len());
    Ok(())
}
```

Supported code attributes:

- `#[contract(code = WALLET_CODE_BOC)]` for a `const &[u8]` or expression such
  as `include_bytes!("wallet.code.boc")`.
- `#[contract(code_hex = "...")]` for inline BoC hex.
- `#[contract(code_file = "wallet.code.boc")]` for `include_bytes!` generated
  by the macro.

`workchain` defaults to `0`. The derived `state_init()` decodes the code BoC
root cell, serializes `data` with `TlbSerialize`, and stores both as referenced
`StateInit` cells. `address()` hashes the serialized `StateInit`, and `bind()`
returns a normal address-bound `Contract<'a, P>`.

Invalid code BoCs, data serialization failures, state-init serialization
failures, and invalid derived configurations are reported as
`ContractBuildError`. The derive rejects unnamed or unit structs, missing
`data`, extra fields, and multiple code sources at compile time.

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
    let state = contract.get_state_decoded_latest().await?;
    let simple = state.simple();
    println!("{:?} {}", simple.state, state.raw.state.len());
    Ok(())
}
```

`get_state_latest` first reads `getMasterchainInfo` and then calls
`getAccountState` for the returned masterchain block. The response preserves the
raw account-state BoC and proof bytes. `get_state_decoded_latest` decodes the
account cell when present, and `get_state_simple_latest` returns a compact state
view with account state and last transaction logical time.

Active-account helpers return `ContractError` when the account is missing,
uninitialized, frozen, or lacks the requested field:

- `active_state_latest()`
- `balance_latest()`
- `code_latest()`
- `data_latest()`

## Get-Methods

```rust
use std::str::FromStr;
use tonutils::contracts::Contract;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::{Address, TvmStack};

async fn example(config_json: &str, address: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let address = Address::from_str(address)?;
    let mut contract = Contract::new(&mut client, address);
    let entries = contract
        .run_get_method_by_name_typed_latest("seqno", TvmStack::empty())
        .await?;
    println!("decoded_stack_entries={}", entries.len());
    Ok(())
}
```

Method names use the standard TON mapping:
`(crc16(method_name) & 0xffff) | 0x10000`. Numeric ids can be passed directly
with `run_get_method` or `run_get_method_latest`. Typed helpers return decoded
stack entries and turn non-zero get-method exit codes into
`ContractError::NonZeroExitCode`.

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

## External Messages And Transactions

`send_external_message_boc(body)` submits an already serialized external
message BoC through LiteAPI `sendMessage` and preserves the bytes exactly. This
is not a wallet signing or deployment builder.

`get_transactions(count, lt, hash)` fetches account transaction history through
the same provider used by the contract wrapper.

`address_from_state_init(workchain, state_init)` is the lower-level primitive
used by `ContractBlueprint::address()`. It serializes the `StateInit` with the
crate TL-B codec, hashes the resulting root cell, and returns the standard
internal address.

## Known Addresses

For already-known addresses, custom clients can own a `Contract<'a, P>` directly.
This is the lower-level escape hatch when code/data address derivation is not
needed.

```rust
use tonutils::contracts::{Contract, ContractProvider};
use tonutils::tvm::Address;

struct MyContract<'a, P: ContractProvider + ?Sized> {
    inner: Contract<'a, P>,
}

impl<'a, P: ContractProvider + ?Sized> MyContract<'a, P> {
    fn new(provider: &'a mut P, address: Address) -> Self {
        Self { inner: Contract::new(provider, address) }
    }
}
```

## Proofs

LiteAPI proof fields are returned as raw bytes. This crate does not verify
account-state, shard, or get-method proofs yet, so callers must not treat these
helpers as a proof-verifying light client API.
