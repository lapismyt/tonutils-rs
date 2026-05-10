//! Generated-backed TL-B codecs for Phase 1 block, config, and proof models.
//!
//! These types cover the cell boundaries and constructor tags needed by
//! LiteClient BoC decoding and offline proof primitive tests. Deep block
//! families that are still generated as raw child cells preserve their exact
//! bytes and references so callers can inspect hashes before opting into
//! verification.

use crate::tlb::{Result, TlbDeserialize, TlbError, TlbSerialize, expect_tag, store_tag};
use crate::tvm::{Builder, Cell, Slice};
use std::sync::Arc;

const BLOCK_TAG: u32 = 0x11ef55aa;
const VALUE_FLOW_TAG: u32 = 0xb8e48dfb;
const VALUE_FLOW_V2_TAG: u32 = 0x3ebf98b7;
const SHARD_STATE_TAG: u32 = 0x9023afe2;
const SPLIT_STATE_TAG: u32 = 0x5f327da5;
const CONFIG_PARAMS_KEY_BITS: usize = 32;

/// TL-B `shard_ident$00 shard_pfx_bits:(#<= 60) workchain_id:int32 shard_prefix:uint64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardIdent {
    /// Number of significant shard-prefix bits.
    pub shard_pfx_bits: u8,
    /// Workchain id.
    pub workchain_id: i32,
    /// Raw 64-bit shard prefix.
    pub shard_prefix: u64,
}

impl TlbSerialize for ShardIdent {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.shard_pfx_bits > 60 {
            return Err(TlbError::CustomSchema {
                schema: "ShardIdent",
                message: format!("shard_pfx_bits {} exceeds 60", self.shard_pfx_bits),
            });
        }
        store_tag(builder, "00")?;
        builder.store_uint(self.shard_pfx_bits as u64, 6)?;
        builder.store_int(self.workchain_id as i64, 32)?;
        builder.store_u64(self.shard_prefix)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardIdent {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag(slice, "ShardIdent", "00")?;
        let shard_pfx_bits = slice.load_uint(6)? as u8;
        if shard_pfx_bits > 60 {
            return Err(TlbError::CustomSchema {
                schema: "ShardIdent",
                message: format!("shard_pfx_bits {shard_pfx_bits} exceeds 60"),
            });
        }
        Ok(Self {
            shard_pfx_bits,
            workchain_id: slice.load_int(32)? as i32,
            shard_prefix: slice.load_u64()?,
        })
    }
}

/// TL-B `ext_blk_ref$_ end_lt:uint64 seq_no:uint32 root_hash:bits256 file_hash:bits256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtBlkRef {
    /// End logical time.
    pub end_lt: u64,
    /// Block sequence number.
    pub seq_no: u32,
    /// Root representation hash.
    pub root_hash: [u8; 32],
    /// File hash.
    pub file_hash: [u8; 32],
}

impl TlbSerialize for ExtBlkRef {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u64(self.end_lt)?;
        builder.store_u32(self.seq_no)?;
        builder.store_bytes(&self.root_hash)?;
        builder.store_bytes(&self.file_hash)?;
        Ok(())
    }
}

impl TlbDeserialize for ExtBlkRef {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            end_lt: slice.load_u64()?,
            seq_no: slice.load_u32()?,
            root_hash: load_hash(slice)?,
            file_hash: load_hash(slice)?,
        })
    }
}

/// TL-B `block_id_ext$_ shard_id:ShardIdent seq_no:uint32 root_hash:bits256 file_hash:bits256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockIdExtTlb {
    /// Shard identifier.
    pub shard_id: ShardIdent,
    /// Block sequence number.
    pub seq_no: u32,
    /// Root representation hash.
    pub root_hash: [u8; 32],
    /// File hash.
    pub file_hash: [u8; 32],
}

impl TlbSerialize for BlockIdExtTlb {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.shard_id.store_tlb(builder)?;
        builder.store_u32(self.seq_no)?;
        builder.store_bytes(&self.root_hash)?;
        builder.store_bytes(&self.file_hash)?;
        Ok(())
    }
}

impl TlbDeserialize for BlockIdExtTlb {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            shard_id: ShardIdent::load_tlb(slice)?,
            seq_no: slice.load_u32()?,
            root_hash: load_hash(slice)?,
            file_hash: load_hash(slice)?,
        })
    }
}

/// TL-B `block#11ef55aa ... = Block`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Global network id.
    pub global_id: i32,
    /// Referenced `BlockInfo` cell.
    pub info: Arc<Cell>,
    /// Referenced `ValueFlow` cell.
    pub value_flow: Arc<Cell>,
    /// Referenced `MERKLE_UPDATE ShardState` cell.
    pub state_update: Arc<Cell>,
    /// Referenced `BlockExtra` cell.
    pub extra: Arc<Cell>,
}

impl TlbSerialize for Block {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(BLOCK_TAG)?;
        builder.store_int(self.global_id as i64, 32)?;
        builder.store_ref(self.info.clone())?;
        builder.store_ref(self.value_flow.clone())?;
        builder.store_ref(self.state_update.clone())?;
        builder.store_ref(self.extra.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for Block {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        load_u32_tag(slice, "Block", BLOCK_TAG)?;
        Ok(Self {
            global_id: slice.load_int(32)? as i32,
            info: slice.load_reference()?,
            value_flow: slice.load_reference()?,
            state_update: slice.load_reference()?,
            extra: slice.load_reference()?,
        })
    }
}

/// TL-B `BlockExtra`, preserved as raw children while generated coverage grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExtra {
    /// Original cell.
    pub cell: Arc<Cell>,
}

impl TlbSerialize for BlockExtra {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.cell)?;
        Ok(())
    }
}

impl TlbDeserialize for BlockExtra {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            cell: consume_remaining_cell(slice)?,
        })
    }
}

/// TL-B `ValueFlow`, preserving either v1 or v2 constructor payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueFlow {
    /// `value_flow#b8e48dfb`.
    V1 { payload: Arc<Cell> },
    /// `value_flow_v2#3ebf98b7`.
    V2 { payload: Arc<Cell> },
}

impl TlbSerialize for ValueFlow {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::V1 { payload } | Self::V2 { payload } => builder.store_cell(payload)?,
        };
        Ok(())
    }
}

impl TlbDeserialize for ValueFlow {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let cell = consume_remaining_cell(slice)?;
        let mut tag_slice = Slice::new(cell.clone());
        let tag = tag_slice.load_u32()?;
        match tag {
            VALUE_FLOW_TAG => Ok(Self::V1 { payload: cell }),
            VALUE_FLOW_V2_TAG => Ok(Self::V2 { payload: cell }),
            _ => Err(TlbError::TagMismatch {
                constructor: "ValueFlow",
                expected_bits: "b8e48dfb|3ebf98b7",
                actual_bits: format!("{tag:08x}"),
            }),
        }
    }
}

/// TL-B `ShardState`, preserving unsplit or split-state payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardState {
    /// `shard_state#9023afe2`.
    Unsplit { payload: Arc<Cell> },
    /// `split_state#5f327da5`.
    Split {
        /// Left shard state.
        left: Arc<Cell>,
        /// Right shard state.
        right: Arc<Cell>,
    },
}

impl TlbSerialize for ShardState {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Unsplit { payload } => {
                builder.store_cell(payload)?;
            }
            Self::Split { left, right } => {
                builder.store_u32(SPLIT_STATE_TAG)?;
                builder.store_ref(left.clone())?;
                builder.store_ref(right.clone())?;
            }
        };
        Ok(())
    }
}

impl TlbDeserialize for ShardState {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let tag = slice.load_u32()?;
        match tag {
            SHARD_STATE_TAG => {
                let mut builder = Builder::new();
                builder.store_u32(SHARD_STATE_TAG)?;
                store_remaining(slice, &mut builder)?;
                Ok(Self::Unsplit {
                    payload: builder.build()?,
                })
            }
            SPLIT_STATE_TAG => Ok(Self::Split {
                left: slice.load_reference()?,
                right: slice.load_reference()?,
            }),
            _ => Err(TlbError::TagMismatch {
                constructor: "ShardState",
                expected_bits: "9023afe2|5f327da5",
                actual_bits: format!("{tag:08x}"),
            }),
        }
    }
}

/// TL-B `_ config_addr:bits256 config:^(Hashmap 32 ^Cell) = ConfigParams`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParams {
    /// Config smart contract address hash.
    pub config_addr: [u8; 32],
    /// Referenced raw `Hashmap 32 ^Cell` config dictionary.
    pub config: Arc<Cell>,
}

impl TlbSerialize for ConfigParams {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_bytes(&self.config_addr)?;
        builder.store_ref(self.config.clone())?;
        Ok(())
    }
}

impl TlbDeserialize for ConfigParams {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let _ = CONFIG_PARAMS_KEY_BITS;
        Ok(Self {
            config_addr: load_hash(slice)?,
            config: slice.load_reference()?,
        })
    }
}

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
            Some(crate::tvm::ExoticCellKind::MerkleProof {
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
            Some(crate::tvm::ExoticCellKind::MerkleUpdate {
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

fn load_hash(slice: &mut Slice) -> Result<[u8; 32]> {
    let mut hash = [0; 32];
    hash.copy_from_slice(&slice.load_bytes(32)?);
    Ok(hash)
}

fn load_u32_tag(slice: &mut Slice, constructor: &'static str, expected: u32) -> Result<()> {
    let actual = slice.load_u32()?;
    if actual == expected {
        Ok(())
    } else {
        Err(TlbError::TagMismatch {
            constructor,
            expected_bits: Box::leak(format!("{expected:08x}").into_boxed_str()),
            actual_bits: format!("{actual:08x}"),
        })
    }
}

fn consume_remaining_cell(slice: &mut Slice) -> Result<Arc<Cell>> {
    let mut builder = Builder::new();
    store_remaining(slice, &mut builder)?;
    Ok(builder.build()?)
}

fn store_remaining(slice: &mut Slice, builder: &mut Builder) -> Result<()> {
    let remaining_bits = slice.remaining_bits();
    if remaining_bits > 0 {
        let bits = slice.load_bits(remaining_bits)?;
        builder.store_bits(&bits, remaining_bits)?;
    }
    for reference in slice.load_remaining_refs()? {
        builder.store_ref(reference)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlb::{TlbDeserialize, TlbSerialize};
    use crate::tvm::Builder;

    #[test]
    fn shard_ident_roundtrips_and_checks_bound() {
        let ident = ShardIdent {
            shard_pfx_bits: 60,
            workchain_id: -1,
            shard_prefix: 0x8000_0000_0000_0000,
        };
        let cell = ident.to_cell().unwrap();
        assert_eq!(ShardIdent::from_cell(cell).unwrap(), ident);

        let invalid = ShardIdent {
            shard_pfx_bits: 61,
            workchain_id: 0,
            shard_prefix: 0,
        };
        assert!(invalid.to_cell().is_err());
    }

    #[test]
    fn block_wrapper_roundtrips_referenced_children() {
        let child = Builder::new().build().unwrap();
        let block = Block {
            global_id: -239,
            info: child.clone(),
            value_flow: child.clone(),
            state_update: child.clone(),
            extra: child,
        };

        let cell = block.to_cell().unwrap();
        let decoded = Block::from_cell(cell.clone()).unwrap();
        assert_eq!(decoded, block);
        assert_eq!(decoded.to_cell().unwrap().hash(), cell.hash());
    }

    #[test]
    fn value_flow_rejects_unknown_constructor() {
        let mut builder = Builder::new();
        builder.store_u32(0xfeed_beef).unwrap();
        let err = ValueFlow::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(err, TlbError::TagMismatch { .. }));
    }
}
