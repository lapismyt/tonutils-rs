//! Runtime facade for the pure-Rust TON SDK.
//!
//! The focused `tonutils-*` crates remain available independently. This crate
//! groups their runtime APIs under one dependency while keeping CLI,
//! procedural macros, and schema-generation tools separate.

#![allow(ambiguous_glob_reexports)]

pub use tonutils_adnl as adnl;
pub use tonutils_contracts as contracts;
pub use tonutils_crc as crc;
pub use tonutils_jetton as jetton;
pub use tonutils_liteclient as liteclient;
pub use tonutils_metadata as metadata;
pub use tonutils_network_config as network_config;
pub use tonutils_nft as nft;
pub use tonutils_tl as tl;
pub use tonutils_tlb as tlb;
pub use tonutils_tvm as tvm;
pub use tonutils_wallet as wallet;

pub use tonutils_crc::*;
pub use tonutils_tl::*;
pub use tonutils_tlb::*;
pub use tonutils_tvm::*;
