mod common;

use tonutils::contracts::Contract;
use tonutils::liteclient::client::LiteClient;
use tonutils::tvm::Address;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = common::load_config()?;
    let mut client = LiteClient::connect_config(&config, common::liteserver_index()?).await?;
    let address = common::contract_address_or_mainnet_default()?;
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
