//! Pure-Rust TON SDK.
//!
//! Features are opt-in. Enable only the protocol layers your application uses.
//!
//! ```toml
//! tonutils = { version = "2", features = ["wallet", "liteclient"] }
//! ```
//! Network trust and proof assumptions remain the responsibility of the
//! application.
//!
//! `tvm` and `tlb` support offline cell and message construction. `liteclient`
//! and `adnl` add network access. `contracts` adds low-level get-method helpers;
//! jetton and NFT modules are not currently included. The CLI, proc-macro, and
//! schema-generation tools remain separate packages.

#![deny(rustdoc::broken_intra_doc_links)]

extern crate self as tonutils;

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
