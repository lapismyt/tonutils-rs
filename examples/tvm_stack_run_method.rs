mod common;

use tonutils::liteclient::client::LiteClient;
use tonutils::tvm::{Address, TvmStack};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(address) = common::get_method_contract_address()? else {
        return Ok(());
    };
    let method = std::env::var("TON_GET_METHOD").unwrap_or_else(|_| "seqno".to_owned());

    let config = common::load_config()?;
    let mut client = LiteClient::connect_config(&config, common::liteserver_index()?).await?;

    let block = client.get_masterchain_info().await?.last;
    let address = Address::from_str(&address)?;

    let result = client
        .run_get_method_by_name(0, block, address, &method, TvmStack::empty())
        .await?;

    println!(
        "method={} exit_code={} result_bytes={}",
        method,
        result.exit_code,
        result.result.as_ref().map_or(0, Vec::len)
    );

    Ok(())
}
