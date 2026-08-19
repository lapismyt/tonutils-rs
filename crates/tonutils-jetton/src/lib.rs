//! TEP-74 jetton payloads, metadata, and get-method decoding.
//!
//! The default build is offline and exposes message/data codecs. Enable the
//! `provider` feature to add helpers for running `get_jetton_data` through
//! `tonutils-contracts`:
//!
//! ```toml
//! tonutils-jetton = { version = "2", features = ["provider"] }
//! ```
//!
//! Metadata parsing is shared with [`tonutils_metadata`](https://docs.rs/tonutils-metadata),
//! and cells are represented by [`tonutils_tvm`](https://docs.rs/tonutils-tvm).
//! Provider calls require a live network and do not constitute proof
//! verification on their own.

#[path = "jetton.rs"]
pub mod jetton;

pub use jetton::*;

#[cfg(any())]
pub(crate) fn method_name_to_id(name: &str) -> u64 {
    let value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((value & 0xFFFF) | 0x10000) as u64
}
