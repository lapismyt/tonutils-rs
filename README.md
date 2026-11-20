# tonutils

[![CI](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml/badge.svg)](https://github.com/lapismyt/tonutils-rs/actions/workflows/live-tests.yml)
[![crates.io](https://img.shields.io/crates/v/tonutils.svg)](https://crates.io/crates/tonutils)
[![codecov](https://codecov.io/gh/lapismyt/tonutils-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/lapismyt/tonutils-rs)
[![tg channel](https://img.shields.io/badge/channel-tonutils__rs-blue?logo=telegram)](https://t.me/tonutils_rs)

> [!NOTE]
> Unfortunately I can't dedicate much time to this project, so most of the code is AI-generated. You can [help the project](#contributing) if you want it to be better.

`tonutils` is a pure-Rust SDK for The Open Network (TON): cells and TL-B,
wallets, jettons and NFTs, a direct liteserver client, and HTTP providers.

## Install

```toml
[dependencies]
tonutils = { version = "2.1", features = ["wallet", "provider", "jetton"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Wallet example

Read the balance and latest transactions, then send TON or jettons:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

use tonutils::{
    jetton::{JettonTransfer, get_wallet_address},
    provider::{GetAccountStateExt, GetTransactionsExt, SendExternalMessageExt, TonApi},
    tvm::Address,
    wallet::{TonMnemonic, WalletOutgoingMessage, WalletV4R2, get_seqno, wallet_v4r2_code},
};

const MNEMONIC: &str = "word1 word2 ... word24";
const JETTON_MASTER: &str = "EQ...";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mnemonic = TonMnemonic::from_phrase(MNEMONIC, None)?;
    let wallet = WalletV4R2::default(
        mnemonic.signing_key().verifying_key().to_bytes(),
        wallet_v4r2_code()?,
        0,
    );
    let address = wallet.address()?;
    let mut provider = TonApi::mainnet();

    // Balance in nanotons.
    let state = provider.get_account_state(address.clone(), None).await?;
    println!("balance: {} nanoTON", state.balance);

    // Latest transactions.
    if let (Some(lt), Some(hash)) = (state.last_transaction_lt, state.last_transaction_hash.clone()) {
        let history = provider.get_transactions(10, address.clone(), lt, hash).await?;
        for tx in history.transactions {
            println!("lt={} utime={:?}", tx.lt, tx.utime);
        }
    }

    let destination = Address::parse("UQ...")?;

    // Send 0.01 TON.
    let ton_message = WalletOutgoingMessage::new(destination.clone(), 10_000_000);
    send(&mut provider, &wallet, &mnemonic, vec![ton_message]).await?;

    // Send 1 jetton (amounts use the jetton's base units).
    let jetton_master = Address::parse(JETTON_MASTER)?;
    let jetton_wallet = get_wallet_address(&mut provider, jetton_master, &address).await?;
    let body = JettonTransfer::new(destination.clone(), 1_000_000, address.clone()).to_cell()?;
    let jetton_message = WalletOutgoingMessage::new(jetton_wallet, 50_000_000).with_body(body);
    send(&mut provider, &wallet, &mnemonic, vec![jetton_message]).await?;

    Ok(())
}

async fn send(
    provider: &mut TonApi,
    wallet: &WalletV4R2,
    mnemonic: &TonMnemonic,
    messages: Vec<WalletOutgoingMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let seqno = get_seqno(provider, wallet.address()?).await?;
    let valid_until = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as u32 + 60;
    let boc = wallet.build_external_message_boc_outgoing(
        seqno,
        valid_until,
        messages,
        mnemonic.signing_key(),
        false,
    )?;
    provider.send_external_message_boc(boc).await?;
    Ok(())
}
```

The example is illustrative: it uses mainnet, has no retry logic, and does not
prove that a liteserver or HTTP provider answered honestly. API documentation
lives on [docs.rs](https://docs.rs/tonutils).

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for
local setup and checks.

You can also support development by sending GRAM or USDT on TON to:

```text
UQAoG1NPcVB66n8HeQq7QiRVWXJZ07Q2Z536blaYROGgD3Cw
```

For every $2 sent, at least one hour will be dedicated to the project through
hands-on, manual development. Add a transfer comment to specify which feature
or area should be prioritized.

## License

Licensed under either of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
