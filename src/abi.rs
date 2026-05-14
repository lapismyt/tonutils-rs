//! ABI data model for TON contract descriptions.
//!
//! This module contains the public Rust model, local structural validation for
//! ABI definitions, and scalar ABI value conversion to and from TVM stack
//! entries. It also provides local get-method stack helpers, message-body
//! codecs, and an optional JSON loader behind `abi-json`. It does not execute
//! contract calls.

mod codec;
mod errors;
#[cfg(feature = "abi-json")]
mod json;
mod model;
#[cfg(test)]
mod tests;

use codec::*;
use errors::*;
#[cfg(feature = "abi-json")]
use json::*;
use model::*;
#[cfg(test)]
use tests::*;

pub use codec::*;
pub use errors::*;
#[cfg(feature = "abi-json")]
pub use json::*;
pub use model::*;
