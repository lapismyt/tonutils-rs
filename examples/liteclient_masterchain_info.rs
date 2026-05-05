use std::str::FromStr;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(config_json) = std::env::var("TON_GLOBAL_CONFIG_JSON").ok() else {
        eprintln!("set TON_GLOBAL_CONFIG_JSON to run this example");
        return Ok(());
    };

    let config = ConfigGlobal::from_str(&config_json)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let info = client.get_masterchain_info().await?;
    println!("masterchain seqno: {}", info.last.seqno);
    Ok(())
}
