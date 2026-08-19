//! TL schema types and LiteAPI serialization.
//!
//! Use this crate for constructor ids, typed LiteAPI request/response values,
//! and raw TL bytes. It is the wire-schema layer below
//! [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient); it does not
//! open sockets or select liteservers.
//!
//! Schema changes should be backed by upstream TON schema evidence and a
//! deterministic round-trip or constructor-id test.
//!
//! This crate is offline and does not open sockets. Pair it with
//! [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient) for live
//! LiteAPI transport and with [`tonutils_tlb`](https://docs.rs/tonutils-tlb)
//! for cell-backed TL-B values.

#[path = "tl/mod.rs"]
pub mod tl;

pub use tl::*;
