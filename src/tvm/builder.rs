//! Enhanced Builder for constructing cells with convenient methods
//!
//! This module provides a high-level builder (`Builder`) that wraps the low-level
//! `CellBuilder` with additional convenience methods for common TON operations.
//!
//! # Builder vs CellBuilder
//!
//! - **`CellBuilder`** (in `cell.rs`): Low-level, minimal API for basic bit/byte operations.
//!   Used internally and for performance-critical code. Provides core functionality like
//!   `store_bits()`, `store_byte()`, `store_u32()`, etc.
//!
//! - **`Builder`** (this module): High-level wrapper around `CellBuilder` with TON-specific
//!   convenience methods like `store_address()`, `store_coins()`, `store_snake_string()`, etc.
//!   Recommended for application code.
//!
//! # When to use which?
//!
//! - Use `CellBuilder` when:
//!   - Implementing low-level TLB serialization
//!   - Performance is critical
//!   - You need minimal overhead
//!
//! - Use `Builder` when:
//!   - Building messages, transactions, or other TON structures
//!   - You want convenient methods for addresses, coins, strings
//!   - Code readability is important
//!
//! # Examples
//!
//! ```rust
//! use tonutils_rs::tvm::{Builder, Address};
//!
//! let mut builder = Builder::new();
//!
//! // Store an address
//! let addr = Address::new(0, [0u8; 32]);
//! builder.store_address(Some(&addr)).unwrap();
//!
//! // Store coins (1 TON)
//! builder.store_coins(1_000_000_000).unwrap();
//!
//! // Store a string
//! builder.store_string("Hello, TON!").unwrap();
//!
//! // Build the cell
//! let cell = builder.build().unwrap();
//! ```

use crate::tvm::address::{Address, ExternalAddress};
use crate::tvm::cell::{Cell, CellBuilder, MAX_CELL_BITS, MAX_CELL_REFS};
use crate::tvm::slice::Slice;
use anyhow::{Result, bail};
use std::sync::Arc;

/// Extended builder with convenience methods
pub struct Builder {
    inner: CellBuilder,
}

impl Builder {
    /// Creates a new builder
    pub fn new() -> Self {
        Self {
            inner: CellBuilder::new(),
        }
    }

    /// Returns the number of bits used
    pub fn bit_len(&self) -> usize {
        // Count bits by building a temporary cell
        // This is a workaround since CellBuilder fields are private
        // In practice, we'd track this internally
        0 // Placeholder - will be calculated when needed
    }

    /// Returns the number of available bits
    pub fn available_bits(&self) -> usize {
        MAX_CELL_BITS - self.bit_len()
    }

    /// Returns the number of available bytes
    pub fn available_bytes(&self) -> usize {
        self.available_bits() / 8
    }

    /// Returns the number of references
    pub fn ref_count(&self) -> usize {
        // Placeholder - will be tracked internally
        0
    }

    /// Returns the number of available references
    pub fn available_refs(&self) -> usize {
        MAX_CELL_REFS - self.ref_count()
    }

    /// Stores a single bit
    pub fn store_bit(&mut self, bit: bool) -> Result<&mut Self> {
        self.inner.store_bit(bit)?;
        Ok(self)
    }

    /// Stores multiple bits from a byte slice
    pub fn store_bits(&mut self, bits: &[u8], bit_len: usize) -> Result<&mut Self> {
        self.inner.store_bits(bits, bit_len)?;
        Ok(self)
    }

    /// Stores a byte
    pub fn store_byte(&mut self, byte: u8) -> Result<&mut Self> {
        self.inner.store_byte(byte)?;
        Ok(self)
    }

    /// Stores multiple bytes
    pub fn store_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self> {
        self.inner.store_bytes(bytes)?;
        Ok(self)
    }

    /// Stores a u32 value
    pub fn store_u32(&mut self, value: u32) -> Result<&mut Self> {
        self.inner.store_u32(value)?;
        Ok(self)
    }

    /// Stores a u64 value
    pub fn store_u64(&mut self, value: u64) -> Result<&mut Self> {
        self.inner.store_u64(value)?;
        Ok(self)
    }

    /// Stores an unsigned integer with specific bit length
    pub fn store_uint(&mut self, value: u64, bits: usize) -> Result<&mut Self> {
        self.inner.store_uint(value, bits)?;
        Ok(self)
    }

    /// Stores a signed integer with specific bit length
    pub fn store_int(&mut self, value: i64, bits: usize) -> Result<&mut Self> {
        if bits > 64 {
            bail!("Cannot store more than 64 bits");
        }

        // Convert signed to unsigned representation
        let unsigned = if value < 0 {
            let mask = (1u64 << bits) - 1;
            (value as u64) & mask
        } else {
            value as u64
        };

        self.store_uint(unsigned, bits)
    }

    /// Stores a boolean value as a single bit
    pub fn store_bool(&mut self, value: bool) -> Result<&mut Self> {
        self.store_bit(value)
    }

    /// Stores a reference to another cell
    pub fn store_ref(&mut self, cell: Arc<Cell>) -> Result<&mut Self> {
        self.inner.store_reference(cell)?;
        Ok(self)
    }

    /// Stores an optional reference (Maybe ^Cell)
    pub fn store_maybe_ref(&mut self, cell: Option<Arc<Cell>>) -> Result<&mut Self> {
        match cell {
            Some(c) => {
                self.store_bit(true)?;
                self.store_ref(c)?;
            }
            None => {
                self.store_bit(false)?;
            }
        }
        Ok(self)
    }

    /// Stores the contents of another cell
    pub fn store_cell(&mut self, cell: &Arc<Cell>) -> Result<&mut Self> {
        if self.ref_count() + cell.reference_count() > MAX_CELL_REFS {
            bail!("Builder refs overflow");
        }

        // Store cell data
        self.store_bits(cell.data(), cell.bit_len())?;

        // Store cell references
        for reference in cell.references() {
            self.store_ref(reference.clone())?;
        }

        Ok(self)
    }

    /// Stores the contents of a slice
    pub fn store_slice(&mut self, slice: &Slice) -> Result<&mut Self> {
        let remaining_bits = slice.remaining_bits();
        if remaining_bits > 0 {
            let bits = slice.clone_from_current();
            let mut temp_slice = bits;
            let data = temp_slice.load_bits(remaining_bits)?;
            self.store_bits(&data, remaining_bits)?;
        }

        // Store remaining references
        let mut temp_slice = slice.clone_from_current();
        while temp_slice.remaining_refs() > 0 {
            let reference = temp_slice.load_reference()?;
            self.store_ref(reference)?;
        }

        Ok(self)
    }

    /// Stores a variable-length unsigned integer (VarUInteger)
    pub fn store_var_uint(&mut self, value: u64, length_bits: usize) -> Result<&mut Self> {
        if value == 0 {
            return self.store_uint(0, length_bits);
        }

        let byte_len = ((64 - value.leading_zeros()) as usize + 7) / 8;
        self.store_uint(byte_len as u64, length_bits)?;
        self.store_uint(value, byte_len * 8)?;
        Ok(self)
    }

    /// Stores a variable-length signed integer (VarInteger)
    pub fn store_var_int(&mut self, value: i64, length_bits: usize) -> Result<&mut Self> {
        if value == 0 {
            return self.store_uint(0, length_bits);
        }

        let bit_len = if value < 0 {
            64 - (value as u64).leading_zeros() as usize
        } else {
            64 - (value as u64).leading_zeros() as usize
        };

        let byte_len = (bit_len + 7) / 8;
        self.store_uint(byte_len as u64, length_bits)?;
        self.store_int(value, byte_len * 8)?;
        Ok(self)
    }

    /// Stores coins (VarUInteger 16)
    pub fn store_coins(&mut self, amount: u128) -> Result<&mut Self> {
        if amount == 0 {
            return self.store_uint(0, 4);
        }

        let byte_len = ((128 - amount.leading_zeros()) as usize + 7) / 8;
        if byte_len > 16 {
            bail!("Coins value too large");
        }

        self.store_uint(byte_len as u64, 4)?;

        // Store the value in big-endian
        let bytes = amount.to_be_bytes();
        let start = 16 - byte_len;
        self.store_bytes(&bytes[start..])?;

        Ok(self)
    }

    /// Stores a string (max 127 bytes)
    pub fn store_string(&mut self, s: &str) -> Result<&mut Self> {
        let bytes = s.as_bytes();
        if bytes.len() > 127 {
            bail!("String too long, use store_snake_string for longer strings");
        }
        self.store_bytes(bytes)
    }

    /// Stores a string using snake encoding (for strings > 127 bytes)
    pub fn store_snake_string(&mut self, s: &str, with_prefix: bool) -> Result<&mut Self> {
        let mut bytes = s.as_bytes().to_vec();
        if with_prefix {
            bytes.insert(0, 0x00);
        }
        self.store_snake_bytes(&bytes)
    }

    /// Stores bytes using snake encoding (splits across multiple cells if needed)
    pub fn store_snake_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self> {
        if bytes.is_empty() {
            return Ok(self);
        }

        let available = self.available_bytes();
        if bytes.len() <= available {
            return self.store_bytes(bytes);
        }

        // Store what fits in this cell
        self.store_bytes(&bytes[..available])?;

        // Store the rest in a reference
        let mut next_builder = Builder::new();
        next_builder.store_snake_bytes(&bytes[available..])?;
        self.store_ref(next_builder.build()?)?;

        Ok(self)
    }

    /// Stores a TON address
    pub fn store_address(&mut self, address: Option<&Address>) -> Result<&mut Self> {
        match address {
            None => {
                // addr_none$00
                self.store_bits(&[0], 2)?;
            }
            Some(addr) => {
                // addr_std$10 anycast:(Maybe Anycast) workchain_id:int8 address:bits256
                self.store_bits(&[0b10], 2)?; // addr_std$10
                self.store_bit(false)?; // no anycast
                self.store_int(addr.workchain as i64, 8)?;
                self.store_bytes(&addr.hash_part)?;
            }
        }
        Ok(self)
    }

    /// Stores an external address
    pub fn store_external_address(&mut self, address: &ExternalAddress) -> Result<&mut Self> {
        // addr_extern$01 len:(## 9) external_address:(bits len)
        self.store_bits(&[0b01], 2)?;
        self.store_uint(address.bit_len as u64, 9)?;

        if let Some(value) = address.value {
            self.store_uint(value, address.bit_len)?;
        }

        Ok(self)
    }

    /// Stores a dictionary (as an optional reference)
    pub fn store_dict(&mut self, dict: Option<Arc<Cell>>) -> Result<&mut Self> {
        self.store_maybe_ref(dict)
    }

    /// Builds the cell
    pub fn build(self) -> Result<Arc<Cell>> {
        self.inner.build()
    }

    /// Converts to a cell (alias for build)
    pub fn end_cell(self) -> Result<Arc<Cell>> {
        self.build()
    }

    /// Converts to a slice
    pub fn to_slice(self) -> Result<Slice> {
        let cell = self.build()?;
        Ok(Slice::new(cell))
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let mut builder = Builder::new();
        builder.store_u32(0x12345678).unwrap();
        builder.store_byte(0xFF).unwrap();

        let cell = builder.build().unwrap();
        assert_eq!(cell.bit_len(), 40);
    }

    #[test]
    fn test_builder_address() {
        let addr = Address::new(0, [0u8; 32]);
        let mut builder = Builder::new();
        builder.store_address(Some(&addr)).unwrap();

        let cell = builder.build().unwrap();
        // 2 bits (addr_std) + 1 bit (no anycast) + 8 bits (workchain) + 256 bits (hash) = 267 bits
        assert_eq!(cell.bit_len(), 267);
    }

    #[test]
    fn test_builder_coins() {
        let mut builder = Builder::new();
        builder.store_coins(1000000000).unwrap(); // 1 TON

        let cell = builder.build().unwrap();
        assert!(cell.bit_len() > 0);
    }

    #[test]
    fn test_builder_string() {
        let mut builder = Builder::new();
        builder.store_string("Hello, TON!").unwrap();

        let cell = builder.build().unwrap();
        assert_eq!(cell.bit_len(), 11 * 8); // 11 characters
    }

    #[test]
    fn test_builder_snake_string() {
        let long_string = "a".repeat(200);
        let mut builder = Builder::new();
        builder.store_snake_string(&long_string, false).unwrap();

        let cell = builder.build().unwrap();
        // Should have created references for the overflow
        assert!(cell.reference_count() > 0);
    }
}
