use tonutils_liteclient::{client::LiteClient, method_name_to_id};
use tonutils_network_config::ConfigGlobal;
use tonutils_tvm::{Address, TvmStack};

const DEFAULT_CONTRACT: &str = "UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP";

fn load_config() -> Option<ConfigGlobal> {
    let json = match std::env::var("TON_GLOBAL_CONFIG_JSON") {
        Ok(json) => json,
        Err(std::env::VarError::NotPresent) => {
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
    let index = std::env::var("TON_LS_INDEX")
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

#[tokio::test]
#[ignore = "requires public liteserver configuration, network access, and a contract"]
async fn live_run_get_method_smoke() {
    let Some(mut client) = connect().await else {
        return;
    };
    let address =
        std::env::var("TON_CONTRACT_ADDRESS").unwrap_or_else(|_| DEFAULT_CONTRACT.to_owned());
    let address = Address::from_str(&address).expect("TON_CONTRACT_ADDRESS must be valid");
    let info = client
        .get_masterchain_info()
        .await
        .expect("getMasterchainInfo must return a typed response");
    let stack = client
        .run_get_method_typed(
            0,
            info.last,
            address,
            method_name_to_id("seqno"),
            TvmStack::empty(),
        )
        .await
        .expect("run_get_method(seqno) must return a successful stack");
    assert!(!stack.is_empty());
}
