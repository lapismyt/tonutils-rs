//! LiteAPI client, liteserver balancing, and contract-facing network helpers.
//!
//! Use [`client::LiteClient`] for one configured liteserver and
//! [`balancer::LiteBalancer`] for
//! retries across several peers. The `network-config` feature (enabled by
//! default) adds parsing of TON global configuration files; disable it when a
//! caller supplies its own peer configuration.
//!
//! ```toml
//! tonutils-liteclient = { version = "2", default-features = false }
//! ```
//!
//! Network clients require live ADNL connectivity and a configured liteserver.
//! Offline BoC, stack, and request construction belongs in
//! [`tonutils_tvm`](https://docs.rs/tonutils-tvm) and
//! [`tonutils_tl`](https://docs.rs/tonutils-tl).

#[path = "liteclient/mod.rs"]
pub mod liteclient;

pub use liteclient::*;

/// Converts a get-method name to the conventional TVM method identifier.
pub fn method_name_to_id(name: &str) -> u64 {
    let method_value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((method_value & 0xFFFF) | 0x10000) as u64
}
