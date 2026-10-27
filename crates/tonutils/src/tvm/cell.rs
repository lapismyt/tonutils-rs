//! Cell implementation for TON blockchain
//!
//! A cell is a fundamental data structure in TON that can store up to 1023 bits
//! of data and maintain up to 4 references to other cells.

#[cfg(test)]
pub(super) use std::sync::Arc;

mod builder;
mod cell;
mod exotic;
#[cfg(test)]
mod tests;

pub use cell::*;
