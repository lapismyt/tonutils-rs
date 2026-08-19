//! Parsing and selecting peers from TON global network configuration files.
//!
//! This crate is offline: it parses caller-provided JSON and does not download
//! configuration or establish network connections. Applications that need the
//! complete live workflow should combine it with
//! [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient).

#[path = "network_config/mod.rs"]
pub mod network_config;

pub use network_config::*;
