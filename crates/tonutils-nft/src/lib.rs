//! NFT payload, metadata, and get-method decoding helpers.

#[path = "nft.rs"]
pub mod nft;

pub use nft::*;

#[cfg(any())]
pub(crate) fn method_name_to_id(name: &str) -> u64 {
    let value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((value & 0xFFFF) | 0x10000) as u64
}
