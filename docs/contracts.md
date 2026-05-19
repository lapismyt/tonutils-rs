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
use tonutils::tvm::Address;

async fn example(config_json: &str, address: &str) -> anyhow::Result<()> {
    let config = ConfigGlobal::from_str(config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let address = Address::from_str(address)?;
    let mut contract = Contract::new(&mut client, address);
    let seqno: u32 = contract
        .run_get_method_by_name_latest_as("seqno", ())
        .await?;
    println!("seqno={seqno}");
    Ok(())
}
```

Method names use the standard TON mapping:
`(crc16(method_name) & 0xffff) | 0x10000`. Numeric ids can be passed directly
with `run_get_method` or `run_get_method_latest`. Raw typed helpers return
decoded stack entries and turn non-zero get-method exit codes into
`ContractError::NonZeroExitCode`.

For wrapper code, prefer the conversion-trait helpers:

```rust
use tonutils::contracts::Contract;
use tonutils::tvm::Address;

async fn wallet_address<P: tonutils::contracts::ContractProvider>(
    contract: &mut Contract<'_, P>,
    owner: Address,
) -> Result<Address, tonutils::contracts::ContractError<P::Error>> {
    contract
        .run_get_method_by_name_latest_as("get_wallet_address", owner)
        .await
}
```

`ToTvmStack`, `FromTvmStack`, `ToTvmStackEntry`, and `FromTvmStackEntry`
cover `()`, raw `TvmStack`, raw entry vectors, stack entries, signed and
unsigned Rust integers, `BigInt`, `BigUint`, `bool`, `Address`, `Arc<Cell>`,
`Option<T>`, and tuples up to eight fields. `Address` values are encoded as
standard internal-address stack slices. `bool` follows the TVM convention:
`-1` is true and `0` is false. Conversion failures are reported as
`ContractError::StackConversion`.

## Result Decoding

`RunMethodResultExt` exposes:

- `raw_result_boc()`: borrowed raw `result` BoC bytes.
- `decode_result_stack()`: attempts to decode supported stack values.
- `result_stack_lossless()`: returns decoded stack values or preserves the raw
  undecodable bytes with the decode error.

The current stack codec supports nulls, integers, cells, slices, tuples, lists,
and explicit unsupported payloads in this crate's internal representation. Stack
compatibility with all liteserver return shapes is still being expanded. The
CLI also exposes ABI get-method argument decoding; generic JSON stack input is
tracked separately.

## ABI Helpers

The `tvm` feature exposes `tonutils::abi` for ABI-driven local encoding and
decoding. `encode_get_method_inputs` converts ABI input values to TVM stack
entries, and `decode_get_method_outputs` converts returned stack entries back
to ABI values according to a `GetMethod` definition. Message helpers
`encode_message_body` and `decode_message_body` support internal and external
message bodies with optional 32-bit opcode prefixes.

`Contract::run_abi_get_method` and `run_abi_get_method_latest` combine those
local codecs with normal get-method execution. `MethodId` selectors are used
directly; functions without a selector use the standard TON method-name id
mapping. `build_abi_external_message_body` and
`build_abi_internal_message_body` build the body cell for ABI message
functions; they do not construct, sign, serialize, or send a full external
message BoC.

Enable `abi-json` to parse ABI documents:

```rust
use tonutils::abi::{AbiSelector, parse_abi_json_str};

fn example(json: &str) -> anyhow::Result<()> {
    let abi = parse_abi_json_str(json)?;
    let method = &abi.contracts[0].methods[0];
    assert!(matches!(method.selector, AbiSelector::MethodId(_)));
    Ok(())
}
```

The `cli` feature includes `abi-json` and exposes ABI get-method invocation:

```bash
tonutils --output json contract run-abi-get-method \
  --address '<addr>' \
  --abi-file contract.abi.json \
  --contract Wallet \
  --method seqno \
  --arg 'owner="0:1111111111111111111111111111111111111111111111111111111111111111"'
```

If `--contract` is omitted, the ABI file must contain exactly one contract. If
`--method` is omitted, the selected contract must contain exactly one
get-method. CLI ABI arguments use `name=json`; map/dictionary values use arrays
of `{ "key": ..., "value": ... }` entries and are limited to fixed-width
integer ABI keys.

## Jetton And NFT Payloads

The `tvm` feature exposes typed message-body builders for common token
workflows:

- `tonutils::jetton::JettonTransferPayload`,
  `JettonBurnPayload`, and `JettonInternalTransferPayload` cover TEP-74
  transfer, burn, and wallet-to-wallet internal transfer bodies.
- `tonutils::nft::NftTransferPayload`,
  `NftOwnershipAssignedPayload`, `NftReportStaticDataPayload`, and
  `NftReportRoyaltyParamsPayload` cover TEP-62 item transfer/static-data
  bodies and TEP-66 royalty reports.
- `inline_forward_payload` and `referenced_forward_payload` select the TL-B
  `Either Cell ^Cell` branch used by forwarded token payloads.

These helpers build body cells only. Use wallet helpers to wrap the body in an
internal message and sign the external wallet request.

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
