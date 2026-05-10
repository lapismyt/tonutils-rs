use std::time::Duration;

mod common;

use tonutils::liteclient::balancer::LiteBalancer;
use tonutils::liteclient::client::LiteClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = common::load_config()?;
    if config.liteservers.is_empty() {
        eprintln!("TON global config contains no liteservers");
        return Ok(());
    }

    let mut peers = Vec::new();
    for liteserver in &config.liteservers {
        match LiteClient::connect_liteserver(liteserver).await {
            Ok(client) => peers.push(client),
            Err(err) => eprintln!("skip liteserver {}: {err}", liteserver.socket_addr()),
        }
    }

    if peers.is_empty() {
        eprintln!("unable to connect to any liteserver from TON global config");
        return Ok(());
    }

    let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
    balancer.start_up().await?;

    let info = balancer.get_masterchain_info().await?;
    println!(
        "masterchain seqno={} alive_peers={} archival_peers={}",
        info.last.seqno,
        balancer.alive_peers_num().await,
        balancer.archival_peers_num().await
    );

    balancer.close_all().await?;
    Ok(())
}
