//! TL-B data models, derives, and cell serialization helpers.
//!
//! TL-B values are encoded into [`tonutils_tvm`](https://docs.rs/tonutils-tvm)
//! cells and slices. Use this crate when defining protocol data models or
//! decoding checked schema slices; use `tonutils-tl` for TL/LiteAPI wire
//! messages. The crate is offline and never contacts the network.
//!
//! Use [`tonutils_tvm`](https://docs.rs/tonutils-tvm) directly for generic cell
//! and BoC operations, and [`tonutils_macros`](https://docs.rs/tonutils-macros)
//! through the optional `tlb-derive` feature for custom model derives.

#[path = "tlb/mod.rs"]
pub mod tlb;

pub use tlb::*;
