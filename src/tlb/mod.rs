//! TL-B model layer.
//!
//! This module provides runtime traits for hand-written TL-B codecs, built-in
//! Phase 1 blockchain models, and a deterministic schema parser/check-summary
//! workflow in [`schema`]. It intentionally does not include a proc-macro derive
//! crate in Phase 1; schema-driven checks and hand-written codecs share the
//! same [`TlbSerialize`] and [`TlbDeserialize`] traits.

pub mod block;
pub mod message;
pub mod schema;
pub mod transaction;

mod bits;
mod core;
#[cfg(test)]
mod fixtures;
mod refs;
#[cfg(test)]
mod tests;
mod varuint;

use bits::*;
use core::*;
#[cfg(test)]
use fixtures::*;
use refs::*;
#[cfg(test)]
use tests::*;
use varuint::*;

pub use bits::*;
pub use core::*;
pub use refs::*;
pub use varuint::*;
