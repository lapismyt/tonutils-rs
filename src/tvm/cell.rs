//! Cell implementation for TON blockchain
//!
//! A cell is a fundamental data structure in TON that can store up to 1023 bits
//! of data and maintain up to 4 references to other cells.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Maximum number of bits a cell can store
pub const MAX_CELL_BITS: usize = 1023;

/// Maximum number of references a cell can have
pub const MAX_CELL_REFS: usize = 4;

/// Cell level range (0-3)
pub const MAX_CELL_LEVEL: u8 = 3;

/// Represents a cell in the TON blockchain
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Cell data as bytes
    data: Vec<u8>,
    /// Number of bits in the cell (not necessarily a multiple of 8)
    bit_len: usize,
    /// References to other cells
    references: Vec<Arc<Cell>>,
    /// Whether this is an exotic (special) cell
    is_exotic: bool,
    /// Cell level (0-3)
    level: u8,
    /// Cached hash
    hash: Option<[u8; 32]>,
    /// Cached depth
    depth: Option<u16>,
}

impl Cell {
    /// Creates a new empty cell
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bit_len: 0,
            references: Vec::new(),
            is_exotic: false,
            level: 0,
            hash: None,
            depth: None,
        }
    }

    /// Creates a cell with the given data and bit length
    pub fn with_data(data: Vec<u8>, bit_len: usize) -> Result<Self> {
        if bit_len > MAX_CELL_BITS {
            bail!(
                "Cell bit length {} exceeds maximum {}",
                bit_len,
                MAX_CELL_BITS
            );
        }

        let required_bytes = (bit_len + 7) / 8;
        if data.len() < required_bytes {
            bail!(
                "Data length {} is insufficient for {} bits",
                data.len(),
                bit_len
            );
        }

        Ok(Self {
            data,
            bit_len,
            references: Vec::new(),
            is_exotic: false,
            level: 0,
            hash: None,
            depth: None,
        })
    }

    /// Adds a reference to another cell
    pub fn add_reference(&mut self, cell: Arc<Cell>) -> Result<()> {
        if self.references.len() >= MAX_CELL_REFS {
            bail!(
                "Cell already has maximum number of references ({})",
                MAX_CELL_REFS
            );
        }
        self.references.push(cell);
        self.hash = None; // Invalidate cached hash
        self.depth = None; // Invalidate cached depth
        self.update_level();
        Ok(())
    }

    /// Returns the cell's data
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the number of bits in the cell
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    /// Returns the cell's references
    pub fn references(&self) -> &[Arc<Cell>] {
        &self.references
    }

    /// Returns whether this is an exotic cell
    pub fn is_exotic(&self) -> bool {
        self.is_exotic
    }

    /// Returns the cell's level
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Updates the cell's level based on its references
    fn update_level(&mut self) {
        if self.is_exotic {
            // Exotic cells have special level calculation rules
            return;
        }

        // For ordinary cells, level is the maximum level of all references
        self.level = self.references.iter().map(|r| r.level()).max().unwrap_or(0);
    }

    /// Computes the cell's descriptors (2 bytes)
    pub fn descriptors(&self) -> [u8; 2] {
        // First byte: r + 8*s + 32*l
        // r = number of references (0-4)
        // s = exotic flag (0 or 1)
        // l = level (0-3)
        let refs_descriptor =
            self.references.len() as u8 + if self.is_exotic { 8 } else { 0 } + self.level * 32;

        // Second byte: floor(b/8) + ceil(b/8)
        // This represents the length of the data
        let bits_descriptor = (self.bit_len / 8 + (self.bit_len + 7) / 8) as u8;

        [refs_descriptor, bits_descriptor]
    }

    /// Serializes the cell data with padding if needed
    pub fn serialize_data(&self) -> Vec<u8> {
        let mut result = self.data.clone();

        // If we have incomplete byte, add padding bit
        if self.bit_len % 8 != 0 {
            let last_byte_idx = self.bit_len / 8;
            if last_byte_idx < result.len() {
                let bits_in_last_byte = self.bit_len % 8;
                // Set the bit after the last data bit to 1 (padding marker)
                result[last_byte_idx] |= 1 << (7 - bits_in_last_byte);
            }
        }

        result
    }

    /// Computes the depth of the cell
    pub fn depth(&self) -> u16 {
        if let Some(d) = self.depth {
            return d;
        }

        let depth = if self.references.is_empty() {
            0
        } else {
            self.references.iter().map(|r| r.depth()).max().unwrap_or(0) + 1
        };

        depth
    }

    /// Computes the representation hash of the cell
    pub fn hash(&self) -> [u8; 32] {
        if let Some(h) = self.hash {
            return h;
        }

        let mut hasher = Sha256::new();

        // 1. Add descriptors
        hasher.update(self.descriptors());

        // 2. Add serialized cell data
        hasher.update(self.serialize_data());

        // 3. Add depth of each reference (2 bytes each)
        for reference in &self.references {
            let depth = reference.depth();
            hasher.update(depth.to_be_bytes());
        }

        // 4. Add hash of each reference
        for reference in &self.references {
            hasher.update(reference.hash());
        }

        // 5. Compute SHA-256
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);

        hash
    }

    /// Returns the number of references
    pub fn reference_count(&self) -> usize {
        self.references.len()
    }

    /// Gets a reference by index
    pub fn reference(&self, index: usize) -> Option<&Arc<Cell>> {
        self.references.get(index)
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

/// Low-level builder for constructing cells
///
/// This is the core, minimal builder that provides basic bit/byte operations.
/// For a higher-level API with TON-specific convenience methods, see [`Builder`](crate::tvm::Builder).
///
/// # When to use CellBuilder
///
/// - Implementing low-level TLB serialization/deserialization
/// - Performance-critical code where minimal overhead is needed
/// - When you only need basic bit/byte operations
///
/// # When to use Builder instead
///
/// - Building messages, transactions, or other TON structures
/// - When you need convenience methods for addresses, coins, strings, etc.
/// - Application-level code where readability is important
///
/// # Example
///
/// ```rust
/// use tonutils_rs::tvm::CellBuilder;
///
/// let mut builder = CellBuilder::new();
/// builder.store_u32(0x12345678).unwrap();
/// builder.store_byte(0xFF).unwrap();
/// let cell = builder.build().unwrap();
/// ```
pub struct CellBuilder {
    data: Vec<u8>,
    bit_len: usize,
    references: Vec<Arc<Cell>>,
}

impl CellBuilder {
    /// Creates a new cell builder
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bit_len: 0,
            references: Vec::new(),
        }
    }

    /// Stores bits from a byte slice
    pub fn store_bits(&mut self, bits: &[u8], bit_len: usize) -> Result<&mut Self> {
        if self.bit_len + bit_len > MAX_CELL_BITS {
            bail!(
                "Cannot store {} bits: would exceed maximum cell size",
                bit_len
            );
        }

        let required_bytes = (bit_len + 7) / 8;
        if bits.len() < required_bytes {
            bail!("Insufficient data for {} bits", bit_len);
        }

        // Append the bits
        for i in 0..bit_len {
            let byte_idx = i / 8;
            let bit_idx = 7 - (i % 8);
            let bit = (bits[byte_idx] >> bit_idx) & 1;

            let target_byte_idx = self.bit_len / 8;
            let target_bit_idx = 7 - (self.bit_len % 8);

            if target_byte_idx >= self.data.len() {
                self.data.push(0);
            }

            if bit == 1 {
                self.data[target_byte_idx] |= 1 << target_bit_idx;
            }

            self.bit_len += 1;
        }

        Ok(self)
    }

    /// Stores a byte
    pub fn store_byte(&mut self, byte: u8) -> Result<&mut Self> {
        self.store_bits(&[byte], 8)
    }

    /// Stores multiple bytes
    pub fn store_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self> {
        self.store_bits(bytes, bytes.len() * 8)
    }

    /// Stores a u32 value
    pub fn store_u32(&mut self, value: u32) -> Result<&mut Self> {
        self.store_bits(&value.to_be_bytes(), 32)
    }

    /// Stores a u64 value
    pub fn store_u64(&mut self, value: u64) -> Result<&mut Self> {
        self.store_bits(&value.to_be_bytes(), 64)
    }

    /// Stores a specific number of bits from a u64
    /// Stores the least significant `bits` of the value in big-endian bit order
    pub fn store_uint(&mut self, value: u64, bits: usize) -> Result<&mut Self> {
        if bits > 64 {
            bail!("Cannot store more than 64 bits from u64");
        }

        // Extract the least significant `bits` from value and store them
        // We need to pack these bits into bytes and then store them with the right bit alignment
        let mut temp = vec![0u8; (bits + 7) / 8];

        // Store bits in big-endian order (MSB first)
        for i in 0..bits {
            if (value & (1u64 << (bits - 1 - i))) != 0 {
                let byte_idx = i / 8;
                let bit_idx = 7 - (i % 8); // MSB first within each byte
                temp[byte_idx] |= 1 << bit_idx;
            }
        }

        self.store_bits(&temp, bits)
    }

    /// Stores a single bit
    pub fn store_bit(&mut self, bit: bool) -> Result<&mut Self> {
        self.store_bits(&[if bit { 0x80 } else { 0x00 }], 1)
    }

    /// Adds a reference to another cell
    pub fn store_reference(&mut self, cell: Arc<Cell>) -> Result<&mut Self> {
        if self.references.len() >= MAX_CELL_REFS {
            bail!(
                "Cannot add reference: maximum {} references allowed",
                MAX_CELL_REFS
            );
        }
        self.references.push(cell);
        Ok(self)
    }

    /// Builds the cell
    pub fn build(self) -> Result<Arc<Cell>> {
        let mut cell = Cell::with_data(self.data, self.bit_len)?;

        for reference in self.references {
            cell.add_reference(reference)?;
        }

        Ok(Arc::new(cell))
    }
}

impl Default for CellBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_cell() {
        let cell = Cell::new();
        assert_eq!(cell.bit_len(), 0);
        assert_eq!(cell.reference_count(), 0);
        assert_eq!(cell.level(), 0);
        assert!(!cell.is_exotic());
    }

    #[test]
    fn test_cell_with_data() {
        let data = vec![0x0F];
        let cell = Cell::with_data(data, 8).unwrap();
        assert_eq!(cell.bit_len(), 8);
        assert_eq!(cell.data()[0], 0x0F);
    }

    #[test]
    fn test_cell_builder() {
        let mut builder = CellBuilder::new();
        builder.store_byte(0xFF).unwrap();
        builder.store_u32(0x12345678).unwrap();

        let cell = builder.build().unwrap();
        assert_eq!(cell.bit_len(), 40); // 8 + 32 bits
    }

    #[test]
    fn test_cell_hash() {
        // Test hash calculation for a simple cell
        let cell = Cell::with_data(vec![0x00, 0x00, 0x00, 0x0F], 32).unwrap();
        let hash = cell.hash();

        // Hash should be 32 bytes
        assert_eq!(hash.len(), 32);

        // Expected hash from documentation example
        let expected =
            hex::decode("57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9")
                .unwrap();
        assert_eq!(&hash[..], &expected[..]);
    }
}
