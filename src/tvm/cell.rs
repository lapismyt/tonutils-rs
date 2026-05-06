//! Cell implementation for TON blockchain
//!
//! A cell is a fundamental data structure in TON that can store up to 1023 bits
//! of data and maintain up to 4 references to other cells.

use anyhow::{Result, bail};
use num_bigint::{BigInt, BigUint};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Maximum number of bits a cell can store
pub const MAX_CELL_BITS: usize = 1023;

/// Maximum number of references a cell can have
pub const MAX_CELL_REFS: usize = 4;

/// Cell level range (0-3)
pub const MAX_CELL_LEVEL: u8 = 3;

/// Supported TON exotic cell kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExoticCellKind {
    /// Pruned branch: tag `0x01`, level mask, stored subtree hashes, and stored depths.
    PrunedBranch {
        level_mask: u8,
        hashes: Vec<[u8; 32]>,
        depths: Vec<u16>,
    },
    /// Library reference: tag `0x02` followed by a referenced library cell hash.
    LibraryReference { hash: [u8; 32] },
    /// Merkle proof: tag `0x03`, deleted subtree hash and depth, and one reference.
    MerkleProof {
        proof_hash: [u8; 32],
        proof_depth: u16,
    },
    /// Merkle update: tag `0x04`, old/new deleted subtree hashes and depths, and two references.
    MerkleUpdate {
        old_hash: [u8; 32],
        new_hash: [u8; 32],
        old_depth: u16,
        new_depth: u16,
    },
}

impl ExoticCellKind {
    /// Returns the TON exotic type tag stored as the first data byte.
    pub fn tag(&self) -> u8 {
        match self {
            Self::PrunedBranch { .. } => 0x01,
            Self::LibraryReference { .. } => 0x02,
            Self::MerkleProof { .. } => 0x03,
            Self::MerkleUpdate { .. } => 0x04,
        }
    }
}

/// Represents a cell in the TON blockchain
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Cell data as bytes
    data: Vec<u8>,
    /// Number of bits in the cell (not necessarily a multiple of 8)
    bit_len: usize,
    /// References to other cells
    references: Vec<Arc<Cell>>,
    /// Exotic cell kind when this is a special cell
    exotic: Option<ExoticCellKind>,
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
            exotic: None,
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

        let mut data = data[..required_bytes].to_vec();
        if bit_len % 8 != 0 {
            let unused_bits = 8 - (bit_len % 8);
            let mask = 0xFFu8 << unused_bits;
            data[required_bytes - 1] &= mask;
        }

        Ok(Self {
            data,
            bit_len,
            references: Vec::new(),
            exotic: None,
            level: 0,
            hash: None,
            depth: None,
        })
    }

    /// Adds a reference to another cell
    pub fn add_reference(&mut self, cell: Arc<Cell>) -> Result<()> {
        if self.is_exotic() {
            bail!("Cannot add references to an already constructed exotic cell");
        }
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
        self.exotic.is_some()
    }

    /// Returns the parsed exotic kind, if this is an exotic cell.
    pub fn exotic_kind(&self) -> Option<&ExoticCellKind> {
        self.exotic.as_ref()
    }

    /// Returns the cell's level
    pub fn level(&self) -> u8 {
        self.level
    }

    /// Updates the cell's level based on its references
    fn update_level(&mut self) {
        if self.is_exotic() {
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
            self.references.len() as u8 + if self.is_exotic() { 8 } else { 0 } + self.level * 32;

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

    /// Creates an exotic cell from raw data and already resolved references.
    pub fn with_exotic_data(
        data: Vec<u8>,
        bit_len: usize,
        references: Vec<Arc<Cell>>,
    ) -> Result<Self> {
        let (data, bit_len) = validate_cell_data(data, bit_len)?;
        if references.len() > MAX_CELL_REFS {
            bail!(
                "Exotic cell has {} references, maximum is {}",
                references.len(),
                MAX_CELL_REFS
            );
        }

        let exotic = parse_exotic_kind(&data, bit_len, &references)?;
        let level = exotic_level(&exotic, &references)?;

        Ok(Self {
            data,
            bit_len,
            references,
            exotic: Some(exotic),
            level,
            hash: None,
            depth: None,
        })
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_cell_data(data: Vec<u8>, bit_len: usize) -> Result<(Vec<u8>, usize)> {
    if bit_len > MAX_CELL_BITS {
        bail!(
            "Cell bit length {} exceeds maximum {}",
            bit_len,
            MAX_CELL_BITS
        );
    }

    let required_bytes = bit_len.div_ceil(8);
    if data.len() < required_bytes {
        bail!(
            "Data length {} is insufficient for {} bits",
            data.len(),
            bit_len
        );
    }

    let mut data = data[..required_bytes].to_vec();
    if bit_len % 8 != 0 {
        let unused_bits = 8 - (bit_len % 8);
        let mask = 0xFFu8 << unused_bits;
        data[required_bytes - 1] &= mask;
    }

    Ok((data, bit_len))
}

fn parse_exotic_kind(
    data: &[u8],
    bit_len: usize,
    references: &[Arc<Cell>],
) -> Result<ExoticCellKind> {
    if bit_len < 8 || data.is_empty() {
        bail!("Invalid exotic cell: missing type tag");
    }

    match data[0] {
        0x01 => parse_pruned_branch(data, bit_len, references),
        0x02 => parse_library_reference(data, bit_len, references),
        0x03 => parse_merkle_proof(data, bit_len, references),
        0x04 => parse_merkle_update(data, bit_len, references),
        tag => bail!("Unsupported exotic cell type: 0x{:02x}", tag),
    }
}

fn parse_pruned_branch(
    data: &[u8],
    bit_len: usize,
    references: &[Arc<Cell>],
) -> Result<ExoticCellKind> {
    if !references.is_empty() {
        bail!("Invalid pruned branch: expected 0 references");
    }
    if bit_len < 16 || data.len() < 2 {
        bail!("Invalid pruned branch: missing level mask");
    }

    let level_mask = data[1];
    if !(1..=7).contains(&level_mask) {
        bail!("Invalid pruned branch level mask: {}", level_mask);
    }

    let hash_count = level_mask.count_ones() as usize;
    let expected_bytes = 2 + hash_count * 32 + hash_count * 2;
    let expected_bits = expected_bytes * 8;
    if bit_len != expected_bits || data.len() != expected_bytes {
        bail!(
            "Invalid pruned branch payload length: expected {} bits, got {}",
            expected_bits,
            bit_len
        );
    }

    let mut hashes = Vec::with_capacity(hash_count);
    let mut pos = 2;
    for _ in 0..hash_count {
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&data[pos..pos + 32]);
        hashes.push(hash);
        pos += 32;
    }

    let mut depths = Vec::with_capacity(hash_count);
    for _ in 0..hash_count {
        depths.push(u16::from_be_bytes([data[pos], data[pos + 1]]));
        pos += 2;
    }

    Ok(ExoticCellKind::PrunedBranch {
        level_mask,
        hashes,
        depths,
    })
}

fn parse_library_reference(
    data: &[u8],
    bit_len: usize,
    references: &[Arc<Cell>],
) -> Result<ExoticCellKind> {
    if !references.is_empty() {
        bail!("Invalid library reference: expected 0 references");
    }
    if bit_len != 264 || data.len() != 33 {
        bail!(
            "Invalid library reference payload length: expected 264 bits, got {}",
            bit_len
        );
    }

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&data[1..33]);
    Ok(ExoticCellKind::LibraryReference { hash })
}

fn parse_merkle_proof(
    data: &[u8],
    bit_len: usize,
    references: &[Arc<Cell>],
) -> Result<ExoticCellKind> {
    if references.len() != 1 {
        bail!(
            "Invalid Merkle proof: expected 1 reference, got {}",
            references.len()
        );
    }
    if bit_len != 280 || data.len() != 35 {
        bail!(
            "Invalid Merkle proof payload length: expected 280 bits, got {}",
            bit_len
        );
    }

    let mut proof_hash = [0u8; 32];
    proof_hash.copy_from_slice(&data[1..33]);
    let proof_depth = u16::from_be_bytes([data[33], data[34]]);
    Ok(ExoticCellKind::MerkleProof {
        proof_hash,
        proof_depth,
    })
}

fn parse_merkle_update(
    data: &[u8],
    bit_len: usize,
    references: &[Arc<Cell>],
) -> Result<ExoticCellKind> {
    if references.len() != 2 {
        bail!(
            "Invalid Merkle update: expected 2 references, got {}",
            references.len()
        );
    }
    if bit_len != 552 || data.len() != 69 {
        bail!(
            "Invalid Merkle update payload length: expected 552 bits, got {}",
            bit_len
        );
    }

    let mut old_hash = [0u8; 32];
    old_hash.copy_from_slice(&data[1..33]);
    let mut new_hash = [0u8; 32];
    new_hash.copy_from_slice(&data[33..65]);
    let old_depth = u16::from_be_bytes([data[65], data[66]]);
    let new_depth = u16::from_be_bytes([data[67], data[68]]);

    Ok(ExoticCellKind::MerkleUpdate {
        old_hash,
        new_hash,
        old_depth,
        new_depth,
    })
}

fn exotic_level(exotic: &ExoticCellKind, references: &[Arc<Cell>]) -> Result<u8> {
    let level = match exotic {
        ExoticCellKind::PrunedBranch { level_mask, .. } => 8 - level_mask.leading_zeros() as u8,
        ExoticCellKind::LibraryReference { .. } => 0,
        ExoticCellKind::MerkleProof { .. } => references[0].level().saturating_sub(1),
        ExoticCellKind::MerkleUpdate { .. } => references
            .iter()
            .map(|reference| reference.level().saturating_sub(1))
            .max()
            .unwrap_or(0),
    };

    if level > MAX_CELL_LEVEL {
        bail!("Invalid exotic cell level: {}", level);
    }

    Ok(level)
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
/// use tonutils::tvm::CellBuilder;
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

    /// Returns the number of bits already stored in this builder.
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    /// Returns the number of references already stored in this builder.
    pub fn ref_count(&self) -> usize {
        self.references.len()
    }

    /// Returns the number of bits that can still be stored in this builder.
    pub fn available_bits(&self) -> usize {
        MAX_CELL_BITS.saturating_sub(self.bit_len)
    }

    /// Returns the number of references that can still be stored in this builder.
    pub fn available_refs(&self) -> usize {
        MAX_CELL_REFS.saturating_sub(self.references.len())
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
        self.store_big_uint(&BigUint::from(value), bits)
    }

    /// Stores an unsigned integer with an exact fixed bit length.
    pub fn store_big_uint(&mut self, value: &BigUint, bits: usize) -> Result<&mut Self> {
        if bits > MAX_CELL_BITS {
            bail!("Cannot store {} bits: exceeds maximum cell size", bits);
        }
        if bits > self.available_bits() {
            bail!(
                "Cannot store {} bits: only {} bits available",
                bits,
                self.available_bits()
            );
        }
        if bits == 0 {
            if value == &BigUint::from(0u8) {
                return Ok(self);
            }
            bail!("Cannot store non-zero unsigned integer in 0 bits");
        }
        if value.bits() as usize > bits {
            bail!("Unsigned integer does not fit in {} bits", bits);
        }

        let value_bytes = value.to_bytes_be();
        let mut temp = vec![0u8; (bits + 7) / 8];
        let start = temp.len().saturating_sub(value_bytes.len());
        temp[start..].copy_from_slice(&value_bytes);

        let unused_low_bits = temp.len() * 8 - bits;
        if unused_low_bits > 0 {
            shift_left_in_place(&mut temp, unused_low_bits);
        }

        self.store_bits(&temp, bits)
    }

    /// Stores a signed integer with an exact fixed bit length using two's complement.
    pub fn store_big_int(&mut self, value: &BigInt, bits: usize) -> Result<&mut Self> {
        if bits > MAX_CELL_BITS {
            bail!("Cannot store {} bits: exceeds maximum cell size", bits);
        }
        if bits == 0 {
            if value == &BigInt::from(0) {
                return Ok(self);
            }
            bail!("Cannot store non-zero signed integer in 0 bits");
        }

        let magnitude = BigInt::from(BigUint::from(1u8) << (bits - 1));
        let min = -magnitude.clone();
        let max = magnitude - BigInt::from(1);
        if value < &min || value > &max {
            bail!("Signed integer does not fit in {} bits", bits);
        }

        let unsigned = if value < &BigInt::from(0) {
            (BigInt::from(BigUint::from(1u8) << bits) + value)
                .to_biguint()
                .ok_or_else(|| anyhow::anyhow!("Invalid two's-complement value"))?
        } else {
            value
                .to_biguint()
                .ok_or_else(|| anyhow::anyhow!("Invalid signed integer value"))?
        };

        self.store_big_uint(&unsigned, bits)
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

fn shift_left_in_place(bytes: &mut [u8], shift: usize) {
    debug_assert!(shift < 8);
    if shift == 0 || bytes.is_empty() {
        return;
    }

    let mut carry = 0u8;
    for byte in bytes.iter_mut().rev() {
        let next_carry = *byte >> (8 - shift);
        *byte = (*byte << shift) | carry;
        carry = next_carry;
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

    fn hex_bytes(hex: &str) -> Vec<u8> {
        hex::decode(hex).unwrap()
    }

    fn representation_preimage(cell: &Cell) -> Vec<u8> {
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&cell.descriptors());
        preimage.extend_from_slice(&cell.serialize_data());

        for reference in cell.references() {
            preimage.extend_from_slice(&reference.depth().to_be_bytes());
        }

        for reference in cell.references() {
            preimage.extend_from_slice(&reference.hash());
        }

        preimage
    }

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
    fn test_cell_with_data_canonicalizes_partial_byte() {
        let cell = Cell::with_data(vec![0xFF, 0xAA], 1).unwrap();
        assert_eq!(cell.bit_len(), 1);
        assert_eq!(cell.data(), &[0x80]);
        assert_eq!(cell.serialize_data(), vec![0xC0]);
    }

    #[test]
    fn test_cell_with_data_preserves_full_byte_data() {
        let cell = Cell::with_data(vec![0xFF, 0xAA], 8).unwrap();
        assert_eq!(cell.data(), &[0xFF]);
        assert_eq!(cell.serialize_data(), vec![0xFF]);
    }

    #[test]
    fn test_cell_descriptors_for_partial_and_full_bytes() {
        assert_eq!(
            Cell::with_data(vec![0x80], 1).unwrap().descriptors(),
            [0, 1]
        );
        assert_eq!(
            Cell::with_data(vec![0xFE], 7).unwrap().descriptors(),
            [0, 1]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF], 8).unwrap().descriptors(),
            [0, 2]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF, 0x80], 9).unwrap().descriptors(),
            [0, 3]
        );
    }

    #[test]
    fn test_cell_serialize_data_top_up_bit() {
        assert_eq!(
            Cell::with_data(vec![0x80], 1).unwrap().serialize_data(),
            vec![0xC0]
        );
        assert_eq!(
            Cell::with_data(vec![0xFE], 7).unwrap().serialize_data(),
            vec![0xFF]
        );
        assert_eq!(
            Cell::with_data(vec![0xAB], 8).unwrap().serialize_data(),
            vec![0xAB]
        );
        assert_eq!(
            Cell::with_data(vec![0xFF, 0x80], 9)
                .unwrap()
                .serialize_data(),
            vec![0xFF, 0xC0]
        );
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
        let cell = Cell::with_data(vec![0x00, 0x00, 0x00, 0x0F], 32).unwrap();

        assert_eq!(representation_preimage(&cell), hex_bytes("00080000000f"));
        assert_eq!(
            cell.hash().as_slice(),
            hex_bytes("57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_golden_fixtures() {
        let fixtures = [
            (
                Cell::new(),
                "0000",
                "96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7",
            ),
            (
                Cell::with_data(vec![0x80], 1).unwrap(),
                "0001c0",
                "7c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0",
            ),
            (
                Cell::with_data(vec![0xAB], 8).unwrap(),
                "0002ab",
                "57c2a1a13baa2762109ed68be0c396f2303ce17e3dde7917d0e74b4072b1dbc7",
            ),
            (
                Cell::with_data(vec![0x00, 0x00, 0x00, 0x0F], 32).unwrap(),
                "00080000000f",
                "57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9",
            ),
        ];

        for (cell, expected_preimage, expected_hash) in fixtures {
            assert_eq!(representation_preimage(&cell), hex_bytes(expected_preimage));
            assert_eq!(cell.hash().as_slice(), hex_bytes(expected_hash));
        }
    }

    #[test]
    fn test_ordinary_cell_hash_two_reference_preimage_order() {
        let first_child = Arc::new(Cell::new());
        let second_child = Arc::new(Cell::with_data(vec![0x80], 1).unwrap());

        let mut root = Cell::with_data(vec![0x80], 1).unwrap();
        root.add_reference(first_child.clone()).unwrap();
        root.add_reference(second_child.clone()).unwrap();

        assert_eq!(first_child.depth(), 0);
        assert_eq!(second_child.depth(), 0);
        assert_eq!(root.depth(), 1);
        assert_eq!(root.descriptors(), [2, 1]);

        let expected_preimage = format!(
            "0201c000000000{}{}",
            hex::encode(first_child.hash()),
            hex::encode(second_child.hash())
        );

        assert_eq!(
            first_child.hash().as_slice(),
            hex_bytes("96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7")
        );
        assert_eq!(
            second_child.hash().as_slice(),
            hex_bytes("7c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0")
        );
        assert_eq!(
            representation_preimage(&root),
            hex_bytes(&expected_preimage)
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("383598f93bde0afbe68b632ae75d5ffa6747df1284e2f4abb86cd2c5840514fe")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_chained_reference_depths() {
        let leaf = Arc::new(Cell::new());

        let mut middle = Cell::with_data(vec![0x80], 1).unwrap();
        middle.add_reference(leaf.clone()).unwrap();
        let middle = Arc::new(middle);

        let mut root = Cell::with_data(vec![0xAB], 8).unwrap();
        root.add_reference(middle.clone()).unwrap();

        assert_eq!(leaf.depth(), 0);
        assert_eq!(middle.depth(), 1);
        assert_eq!(root.depth(), 2);

        assert_eq!(leaf.descriptors(), [0, 0]);
        assert_eq!(middle.descriptors(), [1, 1]);
        assert_eq!(root.descriptors(), [1, 2]);

        assert_eq!(representation_preimage(&leaf), hex_bytes("0000"));
        assert_eq!(
            leaf.hash().as_slice(),
            hex_bytes("96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7")
        );

        assert_eq!(
            representation_preimage(&middle),
            hex_bytes(
                "0101c00000\
                 96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7"
            )
        );
        assert_eq!(
            middle.hash().as_slice(),
            hex_bytes("9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b")
        );

        assert_eq!(
            representation_preimage(&root),
            hex_bytes(
                "0102ab0001\
                 9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b"
            )
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("9f19f1fa052329a70f79c2adaef4e9f4e73eb88be389918473adc5f9a2801181")
        );
    }

    #[test]
    fn test_ordinary_cell_hash_two_reference_depth_order_with_nested_ref() {
        let leaf = Arc::new(Cell::new());

        let mut middle = Cell::with_data(vec![0x80], 1).unwrap();
        middle.add_reference(leaf.clone()).unwrap();
        let middle = Arc::new(middle);

        let mut root = Cell::with_data(vec![0xAB], 8).unwrap();
        root.add_reference(leaf.clone()).unwrap();
        root.add_reference(middle.clone()).unwrap();

        assert_eq!(leaf.depth(), 0);
        assert_eq!(middle.depth(), 1);
        assert_eq!(root.depth(), 2);
        assert_eq!(root.descriptors(), [2, 2]);

        assert_eq!(
            representation_preimage(&root),
            hex_bytes(
                "0202ab00000001\
                 96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7\
                 9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b"
            )
        );
        assert_eq!(
            root.hash().as_slice(),
            hex_bytes("6d112e22e9b4f47922b27cb78ffb8c4c3be4be304cdcb9ad24560e3104827eb6")
        );
    }
}
