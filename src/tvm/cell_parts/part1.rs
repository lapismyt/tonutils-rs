use crate::tvm::uint::UnsignedInteger;
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

    /// Stores a u8 value.
    pub fn store_u8(&mut self, value: u8) -> Result<&mut Self> {
        self.store_uint::<u8>(value)
    }

    /// Stores multiple bytes
    pub fn store_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self> {
        self.store_bits(bytes, bytes.len() * 8)
    }

    /// Stores a u16 value.
    pub fn store_u16(&mut self, value: u16) -> Result<&mut Self> {
        self.store_uint::<u16>(value)
    }

    /// Stores a u32 value
    pub fn store_u32(&mut self, value: u32) -> Result<&mut Self> {
        self.store_uint::<u32>(value)
    }

    /// Stores a u64 value
    pub fn store_u64(&mut self, value: u64) -> Result<&mut Self> {
        self.store_uint::<u64>(value)
    }

    /// Stores an unsigned integer using the natural width of `T`.
    pub fn store_uint<T: UnsignedInteger>(&mut self, value: T) -> Result<&mut Self> {
        self.store_uint_custom(value, T::BITS)
    }

    /// Stores an unsigned integer encoded in `bits` bits.
    pub fn store_uint_custom<T: UnsignedInteger>(
        &mut self,
        value: T,
        bits: usize,
    ) -> Result<&mut Self> {
        if bits > T::BITS {
            bail!(
                "Cannot store {} bits from {}-bit unsigned integer",
                bits,
                T::BITS
            );
        }
        self.store_big_uint(&value.to_big_uint(), bits)
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
