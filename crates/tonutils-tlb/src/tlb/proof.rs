use crate::{Result, TlbError};
use std::sync::Arc;
use tonutils_tvm::Cell;

/// Wrapper for an exotic Merkle proof cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    /// Proof cell.
    pub cell: Arc<Cell>,
    /// Virtual root hash stored in the exotic descriptor.
    pub virtual_hash: [u8; 32],
    /// Virtual root depth stored in the exotic descriptor.
    pub depth: u16,
    /// Referenced virtual root.
    pub virtual_root: Arc<Cell>,
}

impl MerkleProof {
    /// Decodes and validates the proof cell shape without checking trust roots.
    pub fn from_exotic_cell(cell: Arc<Cell>) -> Result<Self> {
        match cell.exotic_kind() {
            Some(tonutils_tvm::ExoticCellKind::MerkleProof {
                proof_hash,
                proof_depth,
            }) if cell.reference_count() == 1 => Ok(Self {
                virtual_hash: *proof_hash,
                depth: *proof_depth,
                virtual_root: cell.references()[0].clone(),
                cell,
            }),
            _ => Err(TlbError::CustomSchema {
                schema: "MERKLE_PROOF",
                message: "expected exotic Merkle proof cell with one reference".to_string(),
            }),
        }
    }

    /// Verifies that the child root hash matches the stored virtual hash.
    pub fn verify_virtual_hash(&self) -> bool {
        self.virtual_root.hash() == self.virtual_hash
    }
}

/// Wrapper for an exotic Merkle update cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleUpdate {
    /// Update cell.
    pub cell: Arc<Cell>,
    /// Old virtual hash.
    pub old_hash: [u8; 32],
    /// New virtual hash.
    pub new_hash: [u8; 32],
    /// Old virtual depth.
    pub old_depth: u16,
    /// New virtual depth.
    pub new_depth: u16,
    /// Old virtual root.
    pub old: Arc<Cell>,
    /// New virtual root.
    pub new: Arc<Cell>,
}

impl MerkleUpdate {
    /// Decodes and validates the update cell shape without checking trust roots.
    pub fn from_exotic_cell(cell: Arc<Cell>) -> Result<Self> {
        match cell.exotic_kind() {
            Some(tonutils_tvm::ExoticCellKind::MerkleUpdate {
                old_hash,
                new_hash,
                old_depth,
                new_depth,
            }) if cell.reference_count() == 2 => Ok(Self {
                old_hash: *old_hash,
                new_hash: *new_hash,
                old_depth: *old_depth,
                new_depth: *new_depth,
                old: cell.references()[0].clone(),
                new: cell.references()[1].clone(),
                cell,
            }),
            _ => Err(TlbError::CustomSchema {
                schema: "MERKLE_UPDATE",
                message: "expected exotic Merkle update cell with two references".to_string(),
            }),
        }
    }

    /// Verifies that child root hashes match the stored virtual hashes.
    pub fn verify_virtual_hashes(&self) -> bool {
        self.old.hash() == self.old_hash && self.new.hash() == self.new_hash
    }
}
