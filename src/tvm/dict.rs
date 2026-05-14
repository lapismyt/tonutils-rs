//! TON HashmapE and HashmapAugE dictionary support.
//!
//! TON dictionaries are canonical Patricia trees over fixed-width bitstring
//! keys. `HashmapE n X` stores either `hme_empty$0` or `hme_root$1` followed by a
//! reference to a `Hashmap n X` edge.

pub(super) use crate::tvm::address::Address;
pub(super) use crate::tvm::builder::Builder;
pub(super) use crate::tvm::cell::Cell;
pub(super) use crate::tvm::slice::Slice;
pub(super) use anyhow::{Result, bail};
pub(super) use std::collections::BTreeMap;
pub(super) use std::sync::Arc;

mod augmented;
mod bit_key;
mod bits;
mod compat;
mod hashmap;
mod labels;
#[cfg(test)]
mod tests;

use augmented::*;
use bit_key::*;
use bits::*;
use compat::*;
use hashmap::*;
use labels::*;
#[cfg(test)]
use tests::*;

pub use augmented::*;
pub use bit_key::*;
pub use bits::*;
pub use compat::*;
pub use hashmap::*;
pub use labels::*;
