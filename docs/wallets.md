# Wallets

`tonutils::wallet` provides offline helpers for Wallet V5R1 and V4R2. The
helpers derive `StateInit` addresses, build signed external message bodies,
serialize external-in message BoCs, and, with `liteclient`, submit those BoCs
through a provider. A submitted BoC is not proof of transaction inclusion.

## Mnemonics

`TonMnemonic` uses 24 English BIP-39 words with TON seed-version checks and
derives an Ed25519 key using TON PBKDF2-HMAC-SHA512 parameters. Optional
mnemonic passwords are supported by the library and CLI through environment
variables, not positional arguments.

```rust
use tonutils::wallet::TonMnemonic;

let mnemonic = TonMnemonic::generate(None)?;
let public_key = mnemonic.public_key();
# Ok::<(), anyhow::Error>(())
```

## Addresses And Transfers

V5R1 is the recommended default. Mainnet V5R1 uses wallet id `0x7fffff11`;
testnet uses `0x7ffffffd`. V4R2 uses the common wallet id `0x29a9a317`.

```rust
use tonutils::wallet::{MAINNET_GLOBAL_ID, WalletV5R1, WalletV5R1WalletId, wallet_v5r1_code};

let wallet_id = WalletV5R1WalletId::client(MAINNET_GLOBAL_ID, 0, 0, 0).pack()?;
let wallet = WalletV5R1::new(public_key, wallet_id, wallet_v5r1_code()?, 0);
let address = wallet.address()?;
# Ok::<(), anyhow::Error>(())
```

`valid_until` is a Unix timestamp stored as `uint32`. `seqno` is replay
protection and must match the current wallet contract state. Include `StateInit`
only for deployment or first-message workflows.

With the `liteclient` feature, `WalletV5R1::send_external_message` and
`WalletV4R2::send_external_message` are accepted LiteAPI submission adapters.
They build and sign an external-in message, optionally include `StateInit` when
`include_state_init` is true, call `ContractProvider::send_external_message_boc`
once, and return the opaque `liteServer.SendMsgStatus.status` value. Provider
errors are surfaced as provider errors, build errors do not call the provider,
and the returned status must not be interpreted as transaction inclusion.

With the `liteclient` feature, `WalletV5R1` also exposes typed get-method
helpers over any `ContractProvider`. The helpers read the latest masterchain
block from the provider, call the deployed wallet address derived from
`WalletV5R1::address()`, and decode successful TVM stack values for `seqno`,
`get_wallet_id`, `get_public_key`, `is_signature_allowed`, and
`get_extensions`.

`extensions_raw_onchain` preserves the exact `get_extensions` cell or slice
payload as `Arc<Cell>`. `extensions_onchain` decodes that payload as
`WalletV5R1Extensions`, a `HashmapE 256 int1` wrapper keyed by 256-bit account
hash. Hash APIs are canonical; address helpers use only `Address::hash_part` and
do not include the workchain in dictionary keys.

Wallet V5R1 extended management actions are available through
`WalletV5R1ExtendedAction` and the explicit `*_with_extended_actions` body/BoC
builders. The ordinary transfer builders still work unchanged and serialize no
extended actions. The V5R1 limit is 255 total ordinary plus extended actions in a
single request.
