//! TEP-62 NFT payloads, collection/item data, and metadata helpers.
//!
//! The default build is offline and suitable for constructing and decoding NFT
//! cells. Enable `provider` for helpers that run collection or item get-methods
//! through `tonutils-contracts`:
//!
//! ```toml
//! tonutils-nft = { version = "2", features = ["provider"] }
//! ```
//!
//! Metadata uses [`tonutils_metadata`](https://docs.rs/tonutils-metadata), and
//! all cell-level values use [`tonutils_tvm`](https://docs.rs/tonutils-tvm).

#[path = "nft.rs"]
pub mod nft;

pub use nft::*;

#[cfg(any())]
pub(crate) fn method_name_to_id(name: &str) -> u64 {
    let value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((value & 0xFFFF) | 0x10000) as u64
}
