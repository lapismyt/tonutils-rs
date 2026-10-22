#[path = "../../examples/src/common.rs"]
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
    let result = contract
        .run_get_method_by_name_latest(&method, TvmStack::empty())
        .await?;

    println!(
        "method={} exit_code={} result_bytes={}",
        method,
        result.exit_code,
        result.result.as_ref().map_or(0, Vec::len)
    );
    Ok(())
}
