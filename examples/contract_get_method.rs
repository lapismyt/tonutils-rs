use std::str::FromStr;
use tonutils::contracts::{Contract, RunMethodResultExt};
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::{Address, TvmStack};

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
    let address = Address::from_str(&address)?;
    let mut contract = Contract::new(&mut client, address);
    let result = contract
        .run_get_method_by_name_latest(&method, TvmStack::empty())
        .await?;

    println!(
        "method={} exit_code={} result_bytes={}",
        method,
        result.exit_code,
        result.raw_result_boc().map_or(0, <[u8]>::len)
    );
    Ok(())
}
