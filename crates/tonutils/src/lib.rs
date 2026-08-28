//! Runtime facade for the pure-Rust TON SDK.
//!
//! The focused `tonutils-*` crates remain available independently. This crate
//! groups their runtime APIs under one dependency while keeping CLI,
//! procedural macros, and schema-generation tools separate.
//!
//! Use the facade when an application spans several runtime layers. Depend on
//! a focused crate when compile time, feature control, or a small dependency
//! graph matters. The default features enable ADNL TCP and network-config
//! support; provider helpers for jettons, NFTs, and wallets are opt-in.
//!
//! ```toml
//! tonutils = { version = "2", features = ["wallet-provider"] }
//! ```
//!
//! The facade re-exports the `adnl`, `contracts`, `crc`, `jetton`, `liteclient`,
//! `metadata`, `network_config`, `nft`, `tl`, `tlb`, `tvm`, and `wallet` modules.
//! Network trust and proof assumptions remain the responsibility of the
//! application.
//!
//! Start with the [project guide](https://lapismyt.github.io/tonutils-rs/) for
//! feature selection and offline/live workflow boundaries. CLI, proc-macro,
//! and schema-generation binaries remain separate packages.

#![allow(ambiguous_glob_reexports)]

pub use tonutils_adnl as adnl;
pub use tonutils_contracts as contracts;
pub use tonutils_crc as crc;
pub use tonutils_jetton as jetton;
pub use tonutils_liteclient as liteclient;
#[cfg(feature = "mempool")]
pub use tonutils_mempool as mempool;
pub use tonutils_metadata as metadata;
pub use tonutils_network_config as network_config;
pub use tonutils_nft as nft;
#[cfg(feature = "overlay")]
pub use tonutils_overlay as overlay;
pub use tonutils_tl as tl;
pub use tonutils_tlb as tlb;
pub use tonutils_tvm as tvm;
pub use tonutils_wallet as wallet;

pub use tonutils_crc::*;
pub use tonutils_tl::*;
pub use tonutils_tlb::*;
pub use tonutils_tvm::*;
