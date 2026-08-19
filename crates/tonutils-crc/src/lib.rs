//! Small CRC helpers used by TON wire formats.
//!
//! The crate is deliberately dependency-light and is useful when implementing
//! constructor identifiers or method ids without pulling in the rest of the
//! SDK. Most applications should use the higher-level APIs in
//! [`tonutils_tl`](https://docs.rs/tonutils-tl) or `tonutils-contracts`.

#[path = "crc/mod.rs"]
pub mod crc;

pub use crc::*;
