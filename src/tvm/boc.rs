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

pub use api::*;
