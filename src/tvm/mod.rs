//! TVM (TON Virtual Machine) data structures and utilities.
//!
//! This module is available with the `tvm` feature and provides the low-level
//! data model used by TL-B codecs, LiteClient BoC helpers, contract
//! get-methods, and offline CLI inspection.
//!
//! Important invariants:
//!
//! - ordinary cells store at most 1023 bits and 4 references;
//! - builders and slices are bounds checked and return errors instead of
//!   truncating data;
//! - BoC decoding preserves supported exotic cell kinds and rejects cache-bit
//!   payloads until the crate has a lossless cache-bit representation;
//! - dictionaries use fixed-width bit keys and canonical label encoding where
//!   the `HashmapE` APIs are used;
//! - TVM stack decoding preserves unsupported payloads for lossless diagnostics.
//!
//! The surface is still expanding toward full TON compatibility. Consult
//! `docs/tvm.md`, `dev-docs/tvm/`, and `TODO.md` before changing wire-format
//! behavior.

pub mod address;
pub mod boc;
pub mod builder;
pub mod cell;
pub mod dict;
pub mod slice;
pub mod stack;
#[cfg(test)]
pub mod tests;
#[doc(hidden)]
pub mod uint;

pub use address::{Address, ExternalAddress};
pub use boc::{
    BocInspection, base64_to_boc, boc_to_base64, boc_to_hex, deserialize_boc,
    deserialize_boc_roots, hex_to_boc, inspect_boc, serialize_boc,
};
pub use builder::Builder;
pub use cell::{Cell, CellBuilder, ExoticCellKind, MAX_CELL_BITS, MAX_CELL_LEVEL, MAX_CELL_REFS};
pub use dict::{
    BitKey, Dict, DictKey, DictValue, HashmapAug, HashmapAugE, HashmapAugFork, HashmapAugLeaf,
    HashmapE,
};
pub use slice::Slice;
pub use stack::{TvmStack, TvmStackEntry};
