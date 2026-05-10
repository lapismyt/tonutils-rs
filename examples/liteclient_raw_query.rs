mod common;

use tonutils::liteclient::client::LiteClient;
use tonutils::tl::Request;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let request = match std::env::var("TON_LITEAPI_REQUEST_HEX") {
        Ok(request_hex) => hex::decode(request_hex.trim())?,
        Err(std::env::VarError::NotPresent) => tl_proto::serialize(Request::GetTime),
        Err(err) => return Err(err.into()),
    };

    let config = common::load_config()?;
    let mut client = LiteClient::connect_config(&config, common::liteserver_index()?).await?;
    let response = client.query_raw(request).await?;
    println!("{}", hex::encode(response));
    Ok(())
}
