//! LiteAPI client, liteserver balancer, and BoC decoding helpers.

#[path = "liteclient/mod.rs"]
pub mod liteclient;

pub use liteclient::*;

/// Converts a get-method name to the conventional TVM method identifier.
pub fn method_name_to_id(name: &str) -> u64 {
    let method_value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((method_value & 0xFFFF) | 0x10000) as u64
}
