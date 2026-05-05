use std::str::FromStr;

use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::{Address, TvmStack, TvmStackEntry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(config_json) = std::env::var("TON_GLOBAL_CONFIG_JSON").ok() else {
        eprintln!("set TON_GLOBAL_CONFIG_JSON to run this example");
        return Ok(());
    };
    let Some(address) = std::env::var("TON_CONTRACT_ADDRESS").ok() else {
        eprintln!("set TON_CONTRACT_ADDRESS to run this example");
        return Ok(());
    };
    let method = std::env::var("TON_GET_METHOD").unwrap_or_else(|_| "seqno".to_owned());

    let config = ConfigGlobal::from_str(&config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;

    let mut stack = TvmStack::empty();
    stack.push(TvmStackEntry::int(0));
    let block = client.get_masterchain_info().await?.last;
    let address = Address::from_str(&address)?;

    let result = client
        .run_get_method_by_name(0, block, address, &method, stack)
        .await?;

    println!(
        "method={} exit_code={} result_bytes={}",
        method,
        result.exit_code,
        result.result.as_ref().map_or(0, Vec::len)
    );

    Ok(())
}
