//! Shared Type Language (TL) primitives and schema infrastructure.
//!
//! This module is available with the `tl` feature and owns common identifiers,
//! serialization helpers, checked-in schema snapshots, generated constructor
//! metadata, and cross-schema tests. Handwritten protocol wire types live with
//! their owning modules: `crate::adnl::tl`, `crate::liteclient::tl`, and
//! `crate::overlay::tl`. Serialization is backed by `tl-proto`; local tests check
//! constructor ids against the checked-in schema snapshots.
//!
//! TL constructor ids are 32-bit little-endian values on the wire. When adding
//! or changing constructors, update the local schema text, keep tests covering
//! flags/vectors/boxed values, and record unsupported upstream fields in
//! `TODO.md` rather than silently omitting them.

pub mod common;
pub mod error;
pub mod generated;
pub mod utils;

#[cfg(test)]
mod tests;

pub use common::{TlInt256, TlString, True};
pub use error::TlError;
