//! TVM (TON Virtual Machine) data structures and utilities
//!
//! This module provides implementations of fundamental TON blockchain data structures:
//! - Cell: The basic data structure that can store up to 1023 bits and up to 4 references
//! - Slice: A reader for sequentially accessing cell data
//! - BoC: Bag of Cells serialization format for encoding cells into byte arrays
//! - Builder: Enhanced builder with convenient methods for common operations
//! - Address: TON address handling (internal and external addresses)
//! - Dict: Dictionary (HashMap) implementation for TON

pub mod address;
pub mod boc;
pub mod builder;
pub mod cell;
pub mod dict;
pub mod slice;
#[cfg(test)]
pub mod tests;

pub use address::{Address, ExternalAddress};
pub use boc::{
    base64_to_boc, boc_to_base64, boc_to_hex, deserialize_boc, hex_to_boc, serialize_boc,
};
pub use builder::Builder;
pub use cell::{Cell, CellBuilder, MAX_CELL_BITS, MAX_CELL_LEVEL, MAX_CELL_REFS};
pub use dict::{Dict, DictKey, DictValue};
pub use slice::Slice;
