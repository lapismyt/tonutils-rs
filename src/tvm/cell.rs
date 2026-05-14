//! Cell implementation for TON blockchain
//!
//! A cell is a fundamental data structure in TON that can store up to 1023 bits
//! of data and maintain up to 4 references to other cells.

pub(super) use crate::tvm::uint::UnsignedInteger;
pub(super) use anyhow::{Result, bail};
pub(super) use num_bigint::{BigInt, BigUint};
pub(super) use sha2::{Digest, Sha256};
pub(super) use std::sync::Arc;

mod builder;
mod cell;
mod exotic;
#[cfg(test)]
mod tests;

use builder::*;
use cell::*;
use exotic::*;
#[cfg(test)]
use tests::*;

pub use builder::*;
pub use cell::*;
pub use exotic::*;
