//! Native Rust TON SDK primitives and LiteAPI clients.
//!
//! `tonutils` is organized as feature-gated layers so embedders can choose
//! low-level protocol primitives without pulling in the network client or CLI.
//! Default features enable `std`, native ADNL TCP, and `LiteClient` support.
//!
//! Public module availability:
//!
//! - `tl`: TL structures and LiteAPI request/response serialization.
//! - `tvm`: cells, slices, builders, BoC helpers, addresses, dictionaries,
//!   TL-B helpers, and TVM stack values.
//! - `adnl` and `adnl-tcp`: ADNL primitives and the native TCP transport.
//! - `liteclient`: LiteAPI client, LiteBalancer, and LiteClient BoC helpers.
//! - `network-config`: TON global config parsing and liteserver extraction.
//! - `cli`: command-line interface support.
//!
//! The crate preserves raw protocol bytes where typed models are incomplete.
//! Proof payloads are not full trust verification unless a specific API says so;
//! current helpers mostly decode, inspect, or preserve proof material for later
//! verification.

#[cfg(feature = "adnl")]
pub mod adnl;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "liteclient")]
pub mod contracts;
#[cfg(feature = "contract-derive")]
pub use tonutils_tlb_derive::Contract;
pub mod crc;
#[cfg(feature = "liteclient")]
pub mod liteclient;
#[cfg(feature = "network-config")]
pub mod network_config;
#[cfg(feature = "tl")]
pub mod tl;
#[cfg(feature = "tvm")]
pub mod tlb;
#[cfg(feature = "tvm")]
pub mod tvm;
pub mod utils;
