//! Slice implementation for reading data from cells
//!
//! A Slice provides a way to read data from a Cell sequentially,
//! tracking the current position in both bits and references.

use crate::tvm::cell::{Cell, MAX_CELL_BITS};
use anyhow::{Result, bail};
use num_bigint::{BigInt, BigUint};
use std::sync::Arc;

/// A slice for reading data from a cell
#[derive(Debug, Clone)]
pub struct Slice {
    /// The cell being read
    cell: Arc<Cell>,
    /// Current bit position in the cell
    bit_pos: usize,
    /// Current reference position
    ref_pos: usize,
}

impl Slice {
    /// Creates a new slice from a cell
    pub fn new(cell: Arc<Cell>) -> Self {
        Self {
            cell,
            bit_pos: 0,
            ref_pos: 0,
        }
    }

    /// Returns the number of remaining bits
    pub fn remaining_bits(&self) -> usize {
        self.cell.bit_len().saturating_sub(self.bit_pos)
    }

    /// Returns the number of remaining references
    pub fn remaining_refs(&self) -> usize {
        self.cell.reference_count().saturating_sub(self.ref_pos)
    }

    /// Checks if there are any remaining bits
    pub fn is_empty(&self) -> bool {
        self.remaining_bits() == 0 && self.remaining_refs() == 0
    }

    /// Loads a single bit
    pub fn load_bit(&mut self) -> Result<bool> {
        if self.remaining_bits() == 0 {
            bail!("No more bits to read");
        }

        let byte_idx = self.bit_pos / 8;
        let bit_idx = 7 - (self.bit_pos % 8);
        let data = self.cell.data();

        if byte_idx >= data.len() {
            bail!("Bit position out of bounds");
        }

        let bit = (data[byte_idx] >> bit_idx) & 1;
        self.bit_pos += 1;

        Ok(bit == 1)
    }

    /// Loads multiple bits into a byte vector
    pub fn load_bits(&mut self, n: usize) -> Result<Vec<u8>> {
        if n > self.remaining_bits() {
            bail!(
                "Not enough bits remaining: requested {}, available {}",
                n,
                self.remaining_bits()
            );
        }

        let mut result = vec![0u8; (n + 7) / 8];

        for i in 0..n {
            let bit = self.load_bit()?;
            if bit {
                let byte_idx = i / 8;
                let bit_idx = 7 - (i % 8);
                result[byte_idx] |= 1 << bit_idx;
            }
        }

        Ok(result)
    }

    /// Loads a byte (8 bits)
    pub fn load_byte(&mut self) -> Result<u8> {
        let bits = self.load_bits(8)?;
        Ok(bits[0])
    }

    /// Loads multiple bytes
    pub fn load_bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        self.load_bits(n * 8)
    }

    /// Loads a u16 value (16 bits, big-endian)
    pub fn load_u16(&mut self) -> Result<u16> {
        let bytes = self.load_bits(16)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Loads a u32 value (32 bits, big-endian)
    pub fn load_u32(&mut self) -> Result<u32> {
        let bytes = self.load_bits(32)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Loads a u64 value (64 bits, big-endian)
    pub fn load_u64(&mut self) -> Result<u64> {
        let bytes = self.load_bits(64)?;
        Ok(u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Loads a uint with a specific number of bits
    pub fn load_uint(&mut self, bits: usize) -> Result<u64> {
        if bits > 64 {
            bail!("Cannot load more than 64 bits into u64");
        }

        let value = self.load_big_uint(bits)?;
        let digits = value.to_u64_digits();
        let result = digits.first().copied().unwrap_or(0);

        Ok(result)
    }

    /// Loads an unsigned big integer with a specific number of bits.
    pub fn load_big_uint(&mut self, bits: usize) -> Result<BigUint> {
        if bits > MAX_CELL_BITS {
            bail!("Cannot load {} bits: exceeds maximum cell size", bits);
        }
        if bits == 0 {
            return Ok(BigUint::from(0u8));
        }

        let bytes = self.load_bits(bits)?;
        let mut value = BigUint::from_bytes_be(&bytes);
        let unused_low_bits = bytes.len() * 8 - bits;
        if unused_low_bits > 0 {
            value >>= unused_low_bits;
        }

        Ok(value)
    }

    /// Loads a signed integer with a specific number of bits
    pub fn load_int(&mut self, bits: usize) -> Result<i64> {
        if bits > 64 {
            bail!("Cannot load more than 64 bits into i64");
        }

        let value = self.load_big_int(bits)?;
        i64::try_from(value).map_err(|_| anyhow::anyhow!("Loaded signed integer does not fit i64"))
    }

    /// Loads a signed big integer with a specific number of bits using two's complement.
    pub fn load_big_int(&mut self, bits: usize) -> Result<BigInt> {
        if bits > MAX_CELL_BITS {
            bail!("Cannot load {} bits: exceeds maximum cell size", bits);
        }
        if bits == 0 {
            return Ok(BigInt::from(0));
        }

        let unsigned = self.load_big_uint(bits)?;
        let sign_bit = BigUint::from(1u8) << (bits - 1);
        if (&unsigned & &sign_bit) != BigUint::from(0u8) {
            Ok(BigInt::from(unsigned) - BigInt::from(BigUint::from(1u8) << bits))
        } else {
            Ok(BigInt::from(unsigned))
        }
    }

    /// Loads a reference to another cell
    pub fn load_reference(&mut self) -> Result<Arc<Cell>> {
        if self.remaining_refs() == 0 {
            bail!("No more references to read");
        }

        let reference = self
            .cell
            .reference(self.ref_pos)
            .ok_or_else(|| anyhow::anyhow!("Reference not found"))?
            .clone();

        self.ref_pos += 1;
        Ok(reference)
    }

    /// Preloads a reference without advancing the position
    pub fn preload_reference(&self, index: usize) -> Result<Arc<Cell>> {
        let actual_index = self.ref_pos + index;
        self.cell
            .reference(actual_index)
            .ok_or_else(|| anyhow::anyhow!("Reference not found at index {}", actual_index))
            .cloned()
    }

    /// Skips a number of bits
    pub fn skip_bits(&mut self, n: usize) -> Result<()> {
        if n > self.remaining_bits() {
            bail!(
                "Cannot skip {} bits: only {} remaining",
                n,
                self.remaining_bits()
            );
        }
        self.bit_pos += n;
        Ok(())
    }

    /// Skips a number of references
    pub fn skip_refs(&mut self, n: usize) -> Result<()> {
        if n > self.remaining_refs() {
            bail!(
                "Cannot skip {} references: only {} remaining",
                n,
                self.remaining_refs()
            );
        }
        self.ref_pos += n;
        Ok(())
    }

    /// Gets the underlying cell
    pub fn cell(&self) -> &Arc<Cell> {
        &self.cell
    }

    /// Gets the current bit position
    pub fn bit_position(&self) -> usize {
        self.bit_pos
    }

    /// Gets the current reference position
    pub fn ref_position(&self) -> usize {
        self.ref_pos
    }

    /// Resets the slice to the beginning
    pub fn reset(&mut self) {
        self.bit_pos = 0;
        self.ref_pos = 0;
    }

    /// Creates a new slice from the current position
    pub fn clone_from_current(&self) -> Self {
        Self {
            cell: self.cell.clone(),
            bit_pos: self.bit_pos,
            ref_pos: self.ref_pos,
        }
    }

    /// Loads all remaining bits
    pub fn load_remaining_bits(&mut self) -> Result<Vec<u8>> {
        let remaining = self.remaining_bits();
        self.load_bits(remaining)
    }

    /// Loads all remaining references
    pub fn load_remaining_refs(&mut self) -> Result<Vec<Arc<Cell>>> {
        let mut refs = Vec::new();
        while self.remaining_refs() > 0 {
            refs.push(self.load_reference()?);
        }
        Ok(refs)
    }

    /// Checks if a specific number of bits can be read
    pub fn can_read_bits(&self, n: usize) -> bool {
        n <= self.remaining_bits()
    }

    /// Checks if a specific number of references can be read
    pub fn can_read_refs(&self, n: usize) -> bool {
        n <= self.remaining_refs()
    }

    /// Loads a variable-length integer (VarUInteger)
    /// First length_bits encode the byte length, then that many bytes of data
    pub fn load_var_uint(&mut self, length_bits: usize) -> Result<u64> {
        let value = self.load_var_big_uint(length_bits)?;
        let digits = value.to_u64_digits();
        if digits.len() > 1 {
            bail!("Loaded VarUInteger does not fit u64");
        }
        Ok(digits.first().copied().unwrap_or(0))
    }

    /// Loads a variable-length unsigned big integer (VarUInteger).
    pub fn load_var_big_uint(&mut self, length_bits: usize) -> Result<BigUint> {
        let byte_len_big = self.load_big_uint(length_bits)?;
        let byte_len_digits = byte_len_big.to_u64_digits();
        if byte_len_digits.len() > 1 {
            bail!("VarUInteger byte length does not fit usize");
        }
        let byte_len = usize::try_from(byte_len_digits.first().copied().unwrap_or(0))
            .map_err(|_| anyhow::anyhow!("VarUInteger byte length does not fit usize"))?;
        if BigUint::from(byte_len) >= (BigUint::from(1u8) << length_bits) {
            bail!(
                "VarUInteger byte length {} does not fit in {} length bits",
                byte_len,
                length_bits
            );
        }
        let value_bits = byte_len
            .checked_mul(8)
            .ok_or_else(|| anyhow::anyhow!("VarUInteger byte length overflow"))?;
        if value_bits > self.remaining_bits() {
            bail!(
                "Not enough bits remaining: requested {}, available {}",
                value_bits,
                self.remaining_bits()
            );
        }

        if byte_len == 0 {
            return Ok(BigUint::from(0u8));
        }

        let bytes = self.load_bytes(byte_len)?;
        Ok(BigUint::from_bytes_be(&bytes))
    }

    /// Loads coins (VarUInteger 16)
    /// Length is encoded in 4 bits, then that many bytes of value
    pub fn load_coins(&mut self) -> Result<u128> {
        // Length encoded in 4 bits (like store_coins in builder)
        let len = self.load_uint(4)? as usize;
        if len > 15 {
            bail!("Coins length {} exceeds maximum 15", len);
        }

        if len == 0 {
            return Ok(0);
        }

        let bytes = self.load_bytes(len)?;
        let mut result = 0u128;
        for &byte in &bytes {
            result = (result << 8) | (byte as u128);
        }

        Ok(result)
    }
}

impl From<Arc<Cell>> for Slice {
    fn from(cell: Arc<Cell>) -> Self {
        Self::new(cell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tvm::cell::CellBuilder;

    #[test]
    fn test_slice_load_bits() {
        let mut builder = CellBuilder::new();
        builder.store_byte(0xFF).unwrap();
        builder.store_byte(0x00).unwrap();
        let cell = builder.build().unwrap();

        let mut slice = Slice::new(cell);
        assert_eq!(slice.remaining_bits(), 16);

        let byte1 = slice.load_byte().unwrap();
        assert_eq!(byte1, 0xFF);
        assert_eq!(slice.remaining_bits(), 8);

        let byte2 = slice.load_byte().unwrap();
        assert_eq!(byte2, 0x00);
        assert_eq!(slice.remaining_bits(), 0);
    }

    #[test]
    fn test_slice_load_uint() {
        let mut builder = CellBuilder::new();
        builder.store_u32(0x12345678).unwrap();
        let cell = builder.build().unwrap();

        let mut slice = Slice::new(cell);
        let value = slice.load_u32().unwrap();
        assert_eq!(value, 0x12345678);
    }

    #[test]
    fn test_slice_load_reference() {
        let ref_cell = CellBuilder::new().build().unwrap();

        let mut builder = CellBuilder::new();
        builder.store_reference(ref_cell.clone()).unwrap();
        let cell = builder.build().unwrap();

        let mut slice = Slice::new(cell);
        assert_eq!(slice.remaining_refs(), 1);

        let _loaded_ref = slice.load_reference().unwrap();
        assert_eq!(slice.remaining_refs(), 0);
    }

    #[test]
    fn test_slice_skip() {
        let mut builder = CellBuilder::new();
        builder.store_u32(0x12345678).unwrap();
        let cell = builder.build().unwrap();

        let mut slice = Slice::new(cell);
        slice.skip_bits(16).unwrap();
        assert_eq!(slice.remaining_bits(), 16);

        let value = slice.load_u16().unwrap();
        assert_eq!(value, 0x5678);
    }
}
