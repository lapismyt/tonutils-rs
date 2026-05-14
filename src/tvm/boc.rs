//! Bag of Cells (BoC) serialization and deserialization
//!
//! BoC is a serialization format that encodes cells into byte arrays.
//! It allows storing and transmitting cell structures efficiently.

pub(super) use crate::tvm::cell::Cell;
pub(super) use anyhow::{Result, bail};
pub(super) use sha2::{Digest, Sha256};
pub(super) use std::collections::HashMap;
pub(super) use std::sync::Arc;

mod api;
mod convert;
mod layout;
mod parse;
mod serialize;
#[cfg(test)]
mod tests;

use api::*;
use convert::*;
use layout::*;
use parse::*;
use serialize::*;
#[cfg(test)]
use tests::*;

pub use api::*;
pub use convert::*;
pub use layout::*;
pub use parse::*;
pub use serialize::*;
