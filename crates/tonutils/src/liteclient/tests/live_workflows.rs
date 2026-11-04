//! Opt-in liteserver smoke tests used by the live CI workflow.

use std::env;

use crate::{liteclient::LiteClient, network_config::ConfigGlobal};

fn load_config() -> ConfigGlobal {
    let json = env::var("TON_GLOBAL_CONFIG_JSON")
        .expect("set TON_GLOBAL_CONFIG_JSON to run this ignored live test");
    json.parse()
        .expect("TON_GLOBAL_CONFIG_JSON must be valid JSON")
}

async fn connect() -> LiteClient {
    let config = load_config();
    let index = env::var("TON_LS_INDEX")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    match LiteClient::connect_config(&config, index).await {
        Ok(client) => client,
        Err(error) => panic!("live liteserver connection failed: {error}"),
    }
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_masterchain_info_smoke() {
    let mut client = connect().await;
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return a typed response");
    assert!(info.last.seqno > 0);
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_version_and_time_smoke() {
    let mut client = connect().await;
    let version = client
        .get_version()
        .await
        .expect("getVersion must return a typed response");
    let now = client
        .get_time()
        .await
        .expect("getTime must return a timestamp");
    assert!(version.version > 0);
    assert!(now > 0);
}

/// Testnet replacement for the PR-blocking DHT gate.
///
/// The canonical testnet DHT seeds are currently unreachable, but the testnet
/// liteservers answer normally. Every configured index is tried because the
/// first entries in the published configuration can be closed while later
/// ones serve the same network.
#[tokio::test]
#[ignore = "requires testnet liteserver configuration and network access"]
async fn live_testnet_liteserver_smoke() {
    let config = load_config();
    let mut failures = Vec::new();
    for index in 0..config.liteservers.len() {
        match LiteClient::connect_config(&config, index).await {
            Ok(mut client) => match client.get_masterchain_info().await {
                Ok(info) => {
                    assert!(info.last.seqno > 0);
                    eprintln!(
                        "testnet liteserver index={index} seqno={} workchain={} shard={}",
                        info.last.seqno, info.last.workchain, info.last.shard
                    );
                    return;
                }
                Err(error) => failures.push(format!("index {index}: {error}")),
            },
            Err(error) => failures.push(format!("index {index}: {error}")),
        }
    }
    panic!(
        "no testnet liteserver answered getMasterchainInfo: {}",
        failures.join(" | ")
    );
}
