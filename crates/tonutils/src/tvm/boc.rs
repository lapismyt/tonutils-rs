//! Bag of Cells (BoC) serialization and deserialization
//!
//! BoC is a serialization format that encodes cells into byte arrays.
//! It allows storing and transmitting cell structures efficiently.

#[cfg(test)]
pub(super) use crate::tvm::cell::Cell;
#[cfg(test)]
pub(super) use std::sync::Arc;

mod api;
mod convert;
mod layout;
mod parse;
mod serialize;
#[cfg(test)]
mod tests;

pub use api::{
    BocInspection, base64_to_boc, boc_to_base64, boc_to_hex, deserialize_boc,
    deserialize_boc_roots, hex_to_boc, inspect_boc, serialize_boc,
};
