//! Opt-in liteserver smoke tests used by the live CI workflow.

use std::env;

use crate::{liteclient::LiteClient, network_config::ConfigGlobal};

fn load_config() -> Option<ConfigGlobal> {
    let json = match env::var("TON_GLOBAL_CONFIG_JSON") {
        Ok(json) => json,
        Err(env::VarError::NotPresent) => {
            eprintln!("skipping live smoke test: TON_GLOBAL_CONFIG_JSON is not set");
            return None;
        }
        Err(error) => panic!("failed to read TON_GLOBAL_CONFIG_JSON: {error}"),
    };

    Some(
        json.parse()
            .expect("TON_GLOBAL_CONFIG_JSON must be valid JSON"),
    )
}

async fn connect() -> Option<LiteClient> {
    let config = load_config()?;
    let index = env::var("TON_LS_INDEX")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    match LiteClient::connect_config(&config, index).await {
        Ok(client) => Some(client),
        Err(error) => panic!("live liteserver connection failed: {error}"),
    }
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_masterchain_info_smoke() {
    let Some(mut client) = connect().await else {
        return;
    };
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return a typed response");
    assert!(info.last.seqno > 0);
}

#[tokio::test]
#[ignore = "requires public liteserver configuration and network access"]
async fn live_get_version_and_time_smoke() {
    let Some(mut client) = connect().await else {
        return;
    };
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
