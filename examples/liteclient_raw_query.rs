use std::str::FromStr;
use tonutils::liteclient::client::LiteClient;
use tonutils::network_config::ConfigGlobal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(config_json) = std::env::var("TON_GLOBAL_CONFIG_JSON").ok() else {
        eprintln!("set TON_GLOBAL_CONFIG_JSON to run this example");
        return Ok(());
    };
    let Some(request_hex) = std::env::var("TON_LITEAPI_REQUEST_HEX").ok() else {
        eprintln!("set TON_LITEAPI_REQUEST_HEX to run this example");
        return Ok(());
    };

    let config = ConfigGlobal::from_str(&config_json)?;
    let request = hex::decode(request_hex)?;
    let mut client = LiteClient::connect_config(&config, 0).await?;
    let response = client.query_raw(request).await?;
    println!("{}", hex::encode(response));
    Ok(())
}
