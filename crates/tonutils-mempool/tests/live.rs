use std::net::SocketAddr;

fn configured_seed(variable: &str) -> SocketAddr {
    let value = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    value
        .parse()
        .unwrap_or_else(|_| panic!("{variable} must contain an IP:port seed"))
}

#[tokio::test]
#[ignore = "requires a configured mainnet seed and live network access"]
async fn mainnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_MAINNET_SEED");
}

#[tokio::test]
#[ignore = "requires a configured testnet seed and live network access"]
async fn testnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_TESTNET_SEED");
}
