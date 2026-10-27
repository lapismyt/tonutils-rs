//! Pure-Rust TON SDK.
//!
//! The default features enable ADNL and network-config
//! support; provider helpers for jettons, NFTs, and wallets are opt-in.
//!
//! ```toml
//! tonutils = { version = "2", features = ["wallet-provider"] }
//! ```
//! Network trust and proof assumptions remain the responsibility of the
//! application.
//!
//! Start with the [project guide](https://lapismyt.github.io/tonutils-rs/) for
//! feature selection and offline/live workflow boundaries. CLI, proc-macro,
//! and schema-generation binaries remain separate packages.

#[cfg(feature = "adnl")]
pub mod adnl;
#[cfg(feature = "contracts")]
pub mod contracts;
#[cfg(feature = "crc")]
pub mod crc;
#[cfg(feature = "liteclient")]
pub mod liteclient;
#[cfg(feature = "mempool")]
pub mod mempool;
#[cfg(feature = "metadata")]
pub mod metadata;
#[cfg(feature = "network-config")]
pub mod network_config;
#[cfg(feature = "overlay")]
pub mod overlay;
#[cfg(feature = "tl")]
pub mod tl;
#[cfg(feature = "tlb")]
pub mod tlb;
#[cfg(feature = "tvm")]
pub mod tvm;
#[cfg(feature = "wallet")]
pub mod wallet;
