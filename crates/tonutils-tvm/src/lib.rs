//! TVM cells, BoC, slices, builders, addresses, dictionaries, and stacks.
//!
//! This is the foundational offline crate in the workspace. A minimal cell
//! workflow uses [`Builder`] to create a [`Cell`], then serializes the root
//! with the BoC helpers exposed by this crate. No liteserver, wallet key, or
//! external configuration is required.
//!
//! The public types are intentionally protocol-oriented: [`Slice`] reads bits
//! and references, dictionaries model Hashmap encodings, and [`TvmStack`]
//! represents get-method values. See the book's TVM chapter for the limits of
//! exotic cells, proof handling, and schema coverage.

#[path = "tvm/mod.rs"]
pub mod tvm;

pub use tvm::*;
