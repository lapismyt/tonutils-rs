//! TON HashmapE and HashmapAugE dictionary support.
//!
//! TON dictionaries are canonical Patricia trees over fixed-width bitstring
//! keys. `HashmapE n X` stores either `hme_empty$0` or `hme_root$1` followed by a
//! reference to a `Hashmap n X` edge.

#[cfg(test)]
pub(super) use crate::tvm::address::Address;
pub(super) use crate::tvm::builder::Builder;
pub(super) use crate::tvm::slice::Slice;
pub(super) use anyhow::{Result, bail};
pub(super) use std::collections::BTreeMap;

mod augmented;
mod bit_key;
mod bits;
mod compat;
mod hashmap;
mod labels;
#[cfg(test)]
mod tests;

use hashmap::*;
use labels::*;

pub use hashmap::*;
