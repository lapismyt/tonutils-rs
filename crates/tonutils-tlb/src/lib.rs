//! TL-B data models, derives, and cell serialization helpers.
//!
//! TL-B values are encoded into [`tonutils_tvm`](https://docs.rs/tonutils-tvm)
//! cells and slices. Use this crate when defining protocol data models or
//! decoding checked schema slices; use `tonutils-tl` for TL/LiteAPI wire
//! messages. The crate is offline and never contacts the network.

#[path = "tlb/mod.rs"]
pub mod tlb;

pub use tlb::*;
