//! Small CRC helpers used by TON wire formats.
//!
//! The crate is deliberately dependency-light and is useful when implementing
//! constructor identifiers or method ids without pulling in the rest of the
//! SDK. Most applications should use the higher-level APIs in
//! [`tonutils_tl`](https://docs.rs/tonutils-tl) or `tonutils-contracts`.
//!
//! This crate has no network behavior or optional feature boundary, making it
//! suitable for small offline tools that only need TON CRC16 or CRC32C values.

#[path = "crc/mod.rs"]
pub mod crc;

pub use crc::*;
