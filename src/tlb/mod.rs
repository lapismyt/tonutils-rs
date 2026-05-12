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

include!("mod_parts/part1.rs");
include!("mod_parts/part2.rs");
include!("mod_parts/part3.rs");
