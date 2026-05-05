use std::str::FromStr;
use tonutils::contracts::Contract;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;
use tonutils::tvm::Address;

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

    let config = ConfigGlobal::from_str(&config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let address = Address::from_str(&address)?;
    let mut contract = Contract::new(&mut client, address);
    let state = contract.get_state_latest().await?;

    println!(
        "block={} shard_block={} state_bytes={}",
        state.id,
        state.shardblk,
        state.state.len()
    );
    Ok(())
}
