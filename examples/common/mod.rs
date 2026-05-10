#![allow(dead_code)]

use std::str::FromStr;

use anyhow::{Context, bail};
use tonutils::network_config::ConfigGlobal;

pub const DEFAULT_MAINNET_CONTRACT_ADDRESS: &str =
    "UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    pub fn from_env() -> anyhow::Result<Self> {
        match std::env::var("TON_NETWORK") {
            Ok(value) if value.eq_ignore_ascii_case("mainnet") => Ok(Self::Mainnet),
            Ok(value) if value.eq_ignore_ascii_case("testnet") => Ok(Self::Testnet),
            Ok(value) => bail!("TON_NETWORK must be either mainnet or testnet, got {value:?}"),
            Err(std::env::VarError::NotPresent) => Ok(Self::Mainnet),
            Err(err) => Err(err).context("failed to read TON_NETWORK"),
        }
    }

    pub fn config_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://ton.org/global.config.json",
            Self::Testnet => "https://ton.org/testnet-global.config.json",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
        }
    }
}

pub fn load_config() -> anyhow::Result<ConfigGlobal> {
    let config_json = match std::env::var("TON_GLOBAL_CONFIG_JSON") {
        Ok(json) => json,
        Err(std::env::VarError::NotPresent) => download_config(Network::from_env()?)?,
        Err(err) => return Err(err).context("failed to read TON_GLOBAL_CONFIG_JSON"),
    };
    ConfigGlobal::from_str(&config_json).context("failed to parse TON global config")
}

pub fn liteserver_index() -> anyhow::Result<usize> {
    match std::env::var("TON_LS_INDEX") {
        Ok(value) => value
            .parse()
            .with_context(|| format!("failed to parse TON_LS_INDEX={value:?} as usize")),
        Err(std::env::VarError::NotPresent) => Ok(0),
        Err(err) => Err(err).context("failed to read TON_LS_INDEX"),
    }
}

pub fn contract_address_or_mainnet_default() -> anyhow::Result<String> {
    match std::env::var("TON_CONTRACT_ADDRESS") {
        Ok(address) => Ok(address),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_MAINNET_CONTRACT_ADDRESS.to_owned()),
        Err(err) => Err(err).context("failed to read TON_CONTRACT_ADDRESS"),
    }
}

pub fn get_method_contract_address() -> anyhow::Result<Option<String>> {
    match std::env::var("TON_CONTRACT_ADDRESS") {
        Ok(address) => Ok(Some(address)),
        Err(std::env::VarError::NotPresent) if Network::from_env()? == Network::Testnet => {
            eprintln!(
                "TON_NETWORK=testnet requires TON_CONTRACT_ADDRESS for get-method examples; \
                 no stable default testnet seqno contract is defined"
            );
            Ok(None)
        }
        Err(std::env::VarError::NotPresent) => {
            Ok(Some(DEFAULT_MAINNET_CONTRACT_ADDRESS.to_owned()))
        }
        Err(err) => Err(err).context("failed to read TON_CONTRACT_ADDRESS"),
    }
}

fn download_config(network: Network) -> anyhow::Result<String> {
    let url = network.config_url();
    eprintln!(
        "TON_GLOBAL_CONFIG_JSON is not set; downloading {} global config from {url}",
        network.name()
    );
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| anyhow::anyhow!("failed to fetch config from {url}: {err:?}"))?;
    if response.status() != 200 {
        bail!("config URL {url} returned HTTP {}", response.status());
    }
    response
        .body_mut()
        .read_to_string()
        .context("failed to read downloaded TON global config")
}
