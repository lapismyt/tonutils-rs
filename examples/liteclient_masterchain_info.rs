mod common;

use tonutils::liteclient::client::LiteClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = common::load_config()?;
    let mut client = LiteClient::connect_config(&config, common::liteserver_index()?).await?;
    let info = client.get_masterchain_info().await?;
    println!("masterchain seqno: {}", info.last.seqno);
    Ok(())
}
