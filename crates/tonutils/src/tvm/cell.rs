//! Cell implementation for TON blockchain
//!
//! A cell is a fundamental data structure in TON that can store up to 1023 bits
//! of data and maintain up to 4 references to other cells.

#[cfg(test)]
pub(super) use std::sync::Arc;

mod builder;
#[expect(
    clippy::module_inception,
    reason = "The cell owner keeps its implementation in a sibling module of the same name"
)]
mod cell;
mod exotic;
#[cfg(test)]
mod tests;

pub use cell::{Cell, CellBuilder, ExoticCellKind, MAX_CELL_BITS, MAX_CELL_LEVEL, MAX_CELL_REFS};
