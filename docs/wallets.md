# Wallets

`tonutils::wallet` provides offline helpers for Wallet V5R1 and V4R2. The
helpers derive `StateInit` addresses, build signed external message bodies, and
serialize external-in message BoCs. Network submission is still a LiteClient or
LiteBalancer operation; a submitted BoC is not proof of transaction inclusion.

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
