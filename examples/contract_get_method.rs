mod common;

use tonutils_contracts::Contract;
use tonutils_liteclient::client::LiteClient;
use tonutils_tvm::{Address, TvmStack};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(address) = common::contract_address()? else {
        return Ok(());
    };
    let method = std::env::var("TON_GET_METHOD").unwrap_or_else(|_| "seqno".to_owned());

    let config = common::load_config()?;
    let mut client = LiteClient::connect_config(&config, common::liteserver_index()?).await?;
    let address = Address::from_str(&address)?;
    let mut contract = Contract::new(&mut client, address);
    let entries = contract
        .run_get_method_by_name_typed_latest(&method, TvmStack::empty())
        .await?;

    println!("method={} decoded_stack_entries={}", method, entries.len());
    Ok(())
}
