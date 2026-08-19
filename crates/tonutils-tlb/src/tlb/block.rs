//! Generated-backed TL-B codecs for Initial compatibility block, config, and proof models.
//!
//! These types cover the cell boundaries and constructor tags needed by
//! LiteClient BoC decoding and offline proof primitive tests. Deep block
//! families that are still generated as raw child cells preserve their exact
//! bytes and references so callers can inspect hashes before opting into
//! verification.

use crate::{
    CurrencyCollection, Result, ShardAccounts, TlbDeserialize, TlbError, TlbSerialize, expect_tag,
    load_ref_tlb, store_ref_tlb, store_tag,
};
use std::sync::Arc;
use tonutils_tvm::{BitKey, Builder, Cell, HashmapE, Slice};

#[path = "proof.rs"]
mod proof;
pub use proof::{MerkleProof, MerkleUpdate};

const BLOCK_TAG: u32 = 0x11ef55aa;
const VALUE_FLOW_TAG: u32 = 0xb8e48dfb;
const VALUE_FLOW_V2_TAG: u32 = 0x3ebf98b7;
const SHARD_STATE_TAG: u32 = 0x9023afe2;
const SPLIT_STATE_TAG: u32 = 0x5f327da5;
const BLOCK_INFO_TAG: u32 = 0x9bc7a987;
const GLOBAL_VERSION_TAG: u8 = 0xc4;
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
        builder.store_uint_custom::<u8>(self.shard_pfx_bits, 6)?;
        builder.store_int(self.workchain_id as i64, 32)?;
        builder.store_u64(self.shard_prefix)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardIdent {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag(slice, "ShardIdent", "00")?;
        let shard_pfx_bits = slice.load_uint_custom::<u8>(6)?;
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

/// TL-B `master_info$_ master:ExtBlkRef = BlkMasterInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlkMasterInfo {
    /// Referenced masterchain block.
    pub master: ExtBlkRef,
}

impl TlbSerialize for BlkMasterInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.master.store_tlb(builder)
    }
}

impl TlbDeserialize for BlkMasterInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            master: ExtBlkRef::load_tlb(slice)?,
        })
    }
}

/// TL-B `capabilities#c4 version:uint32 capabilities:uint64 = GlobalVersion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalVersion {
    /// Protocol version.
    pub version: u32,
    /// Capability bitset.
    pub capabilities: u64,
}

impl TlbSerialize for GlobalVersion {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_uint(GLOBAL_VERSION_TAG)?;
        builder.store_u32(self.version)?;
        builder.store_u64(self.capabilities)?;
        Ok(())
    }
}

impl TlbDeserialize for GlobalVersion {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        if slice.load_uint::<u8>()? != GLOBAL_VERSION_TAG {
            return Err(TlbError::TagMismatch {
                constructor: "GlobalVersion",
                expected_bits: "c4",
                actual_bits: "different".to_string(),
            });
        }
        Ok(Self {
            version: slice.load_u32()?,
            capabilities: slice.load_u64()?,
        })
    }
}

/// TL-B `BlockPrevInfo`, selected by the `after_merge` flag in `BlockInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockPrevInfo {
    /// `prev_blk_info$_ prev:ExtBlkRef = BlkPrevInfo 0`.
    Single { prev: ExtBlkRef },
    /// `prev_blks_info$_ prev1:^ExtBlkRef prev2:^ExtBlkRef = BlkPrevInfo 1`.
    Split { prev1: ExtBlkRef, prev2: ExtBlkRef },
}

impl TlbSerialize for BlockPrevInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::Single { prev } => prev.store_tlb(builder),
            Self::Split { prev1, prev2 } => {
                store_ref_tlb(builder, prev1)?;
                store_ref_tlb(builder, prev2)
            }
        }
    }
}

impl TlbDeserialize for BlockPrevInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        if slice.remaining_refs() == 0 {
            return Ok(Self::Single {
                prev: ExtBlkRef::load_tlb(slice)?,
            });
        }
        if slice.remaining_refs() == 2 && slice.remaining_bits() == 0 {
            return Ok(Self::Split {
                prev1: load_ref_tlb(slice, "ExtBlkRef")?,
                prev2: load_ref_tlb(slice, "ExtBlkRef")?,
            });
        }
        Err(TlbError::CustomSchema {
            schema: "BlockPrevInfo",
            message: "invalid predecessor reference layout".to_string(),
        })
    }
}

/// TL-B `block_info#9bc7a987 ... = BlockInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// Block format version.
    pub version: u32,
    /// Whether this is not a masterchain block.
    pub not_master: bool,
    /// Whether the block follows a merge.
    pub after_merge: bool,
    /// Whether the block precedes a split.
    pub before_split: bool,
    /// Whether the block follows a split.
    pub after_split: bool,
    /// Whether a split is requested.
    pub want_split: bool,
    /// Whether a merge is requested.
    pub want_merge: bool,
    /// Whether this is a key block.
    pub key_block: bool,
    /// Whether the vertical sequence number increments.
    pub vert_seqno_incr: bool,
    /// Block flags; upstream currently constrains this to zero or one.
    pub flags: u8,
    /// Horizontal sequence number.
    pub seq_no: u32,
    /// Vertical sequence number.
    pub vert_seq_no: u32,
    /// Shard identifier.
    pub shard: ShardIdent,
    /// Generation Unix timestamp.
    pub gen_utime: u32,
    /// Start logical time.
    pub start_lt: u64,
    /// End logical time.
    pub end_lt: u64,
    /// Short validator-list hash.
    pub gen_validator_list_hash_short: u32,
    /// Catchain sequence number.
    pub gen_catchain_seqno: u32,
    /// Minimum referenced masterchain sequence number.
    pub min_ref_mc_seqno: u32,
    /// Previous key block sequence number.
    pub prev_key_block_seqno: u32,
    /// Optional software version, present when flag bit zero is clear.
    pub gen_software: Option<GlobalVersion>,
    /// Optional masterchain reference for shardchain blocks.
    pub master_ref: Option<BlkMasterInfo>,
    /// Previous block reference selected by `after_merge`.
    pub prev_ref: BlockPrevInfo,
    /// Optional previous vertical block reference.
    pub prev_vert_ref: Option<ExtBlkRef>,
}

impl TlbSerialize for BlockInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        if self.flags > 1 {
            return Err(TlbError::NonCanonicalValue {
                schema: "BlockInfo",
                reason: "flags must be at most one".to_string(),
            });
        }
        builder.store_u32(BLOCK_INFO_TAG)?;
        builder.store_u32(self.version)?;
        builder.store_bit(self.not_master)?;
        builder.store_bit(self.after_merge)?;
        builder.store_bit(self.before_split)?;
        builder.store_bit(self.after_split)?;
        builder.store_bit(self.want_split)?;
        builder.store_bit(self.want_merge)?;
        builder.store_bit(self.key_block)?;
        builder.store_bit(self.vert_seqno_incr)?;
        builder.store_uint(self.flags)?;
        builder.store_uint(self.seq_no)?;
        builder.store_uint(self.vert_seq_no)?;
        self.shard.store_tlb(builder)?;
        builder.store_u32(self.gen_utime)?;
        builder.store_u64(self.start_lt)?;
        builder.store_u64(self.end_lt)?;
        builder.store_u32(self.gen_validator_list_hash_short)?;
        builder.store_u32(self.gen_catchain_seqno)?;
        builder.store_u32(self.min_ref_mc_seqno)?;
        builder.store_u32(self.prev_key_block_seqno)?;
        if self.flags & 1 == 0 {
            let software = self.gen_software.as_ref().ok_or(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "gen_software is required when flags bit zero is clear".to_string(),
            })?;
            software.store_tlb(builder)?;
        } else if self.gen_software.is_some() {
            return Err(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "gen_software must be absent when flags bit zero is set".to_string(),
            });
        }
        if self.not_master {
            let master_ref = self.master_ref.as_ref().ok_or(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "master_ref is required for a non-master block".to_string(),
            })?;
            store_ref_tlb(builder, master_ref)?;
        } else if self.master_ref.is_some() {
            return Err(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "master_ref must be absent for a master block".to_string(),
            });
        }
        store_ref_tlb(builder, &self.prev_ref)?;
        if self.vert_seqno_incr {
            let prev_vert_ref = self.prev_vert_ref.as_ref().ok_or(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "prev_vert_ref is required when vert_seqno_incr is set".to_string(),
            })?;
            store_ref_tlb(builder, prev_vert_ref)?;
        } else if self.prev_vert_ref.is_some() {
            return Err(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "prev_vert_ref must be absent when vert_seqno_incr is clear".to_string(),
            });
        }
        Ok(())
    }
}

impl TlbDeserialize for BlockInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        load_u32_tag(slice, "BlockInfo", BLOCK_INFO_TAG)?;
        let version = slice.load_u32()?;
        let not_master = slice.load_bit()?;
        let after_merge = slice.load_bit()?;
        let before_split = slice.load_bit()?;
        let after_split = slice.load_bit()?;
        let want_split = slice.load_bit()?;
        let want_merge = slice.load_bit()?;
        let key_block = slice.load_bit()?;
        let vert_seqno_incr = slice.load_bit()?;
        let flags = slice.load_uint::<u8>()?;
        if flags > 1 {
            return Err(TlbError::NonCanonicalValue {
                schema: "BlockInfo",
                reason: "flags must be at most one".to_string(),
            });
        }
        let seq_no = slice.load_uint::<u32>()?;
        let vert_seq_no = slice.load_uint::<u32>()?;
        let shard = ShardIdent::load_tlb(slice)?;
        let gen_utime = slice.load_u32()?;
        let start_lt = slice.load_u64()?;
        let end_lt = slice.load_u64()?;
        let gen_validator_list_hash_short = slice.load_u32()?;
        let gen_catchain_seqno = slice.load_u32()?;
        let min_ref_mc_seqno = slice.load_u32()?;
        let prev_key_block_seqno = slice.load_u32()?;
        let gen_software = if flags & 1 == 0 {
            Some(GlobalVersion::load_tlb(slice)?)
        } else {
            None
        };
        let master_ref = if not_master {
            Some(load_ref_tlb(slice, "BlkMasterInfo")?)
        } else {
            None
        };
        let prev_ref = load_ref_tlb(slice, "BlockPrevInfo")?;
        let prev_vert_ref = if vert_seqno_incr {
            Some(load_ref_tlb(slice, "ExtBlkRef")?)
        } else {
            None
        };
        if matches!(
            (&prev_ref, after_merge),
            (BlockPrevInfo::Split { .. }, false) | (BlockPrevInfo::Single { .. }, true)
        ) {
            return Err(TlbError::CustomSchema {
                schema: "BlockInfo",
                message: "predecessor branch does not match after_merge".to_string(),
            });
        }
        Ok(Self {
            version,
            not_master,
            after_merge,
            before_split,
            after_split,
            want_split,
            want_merge,
            key_block,
            vert_seqno_incr,
            flags,
            seq_no,
            vert_seq_no,
            shard,
            gen_utime,
            start_lt,
            end_lt,
            gen_validator_list_hash_short,
            gen_catchain_seqno,
            min_ref_mc_seqno,
            prev_key_block_seqno,
            gen_software,
            master_ref,
            prev_ref,
            prev_vert_ref,
        })
    }
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

/// TL-B `block_extra$_ ... = BlockExtra`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExtra {
    /// Incoming-message descriptor cell.
    pub in_msg_descr: Arc<Cell>,
    /// Outgoing-message descriptor cell.
    pub out_msg_descr: Arc<Cell>,
    /// Account-block dictionary cell.
    pub account_blocks: Arc<Cell>,
    /// Block random seed.
    pub rand_seed: [u8; 32],
    /// Creator hash.
    pub created_by: [u8; 32],
    /// Optional masterchain extra data.
    pub custom: Option<McBlockExtra>,
}

/// TL-B `McBlockExtra` payload preserved as a raw cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McBlockExtra {
    /// Original `McBlockExtra` cell.
    pub cell: Arc<Cell>,
}

impl TlbSerialize for McBlockExtra {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.cell)?;
        Ok(())
    }
}

impl TlbDeserialize for McBlockExtra {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            cell: consume_remaining_cell(slice)?,
        })
    }
}

impl TlbSerialize for BlockExtra {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_ref(self.in_msg_descr.clone())?;
        builder.store_ref(self.out_msg_descr.clone())?;
        builder.store_ref(self.account_blocks.clone())?;
        builder.store_bytes(&self.rand_seed)?;
        builder.store_bytes(&self.created_by)?;
        store_maybe_ref(builder, &self.custom)?;
        Ok(())
    }
}

impl TlbDeserialize for BlockExtra {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            in_msg_descr: slice.load_reference()?,
            out_msg_descr: slice.load_reference()?,
            account_blocks: slice.load_reference()?,
            rand_seed: load_hash(slice)?,
            created_by: load_hash(slice)?,
            custom: load_maybe_ref(slice, "McBlockExtra")?,
        })
    }
}

/// The first referenced value group in `ValueFlow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueFlowMain {
    /// Value carried from the previous block.
    pub from_prev_blk: CurrencyCollection,
    /// Value carried to the next block.
    pub to_next_blk: CurrencyCollection,
    /// Imported value.
    pub imported: CurrencyCollection,
    /// Exported value.
    pub exported: CurrencyCollection,
}

impl TlbSerialize for ValueFlowMain {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.from_prev_blk.store_tlb(builder)?;
        self.to_next_blk.store_tlb(builder)?;
        self.imported.store_tlb(builder)?;
        self.exported.store_tlb(builder)
    }
}

impl TlbDeserialize for ValueFlowMain {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            from_prev_blk: CurrencyCollection::load_tlb(slice)?,
            to_next_blk: CurrencyCollection::load_tlb(slice)?,
            imported: CurrencyCollection::load_tlb(slice)?,
            exported: CurrencyCollection::load_tlb(slice)?,
        })
    }
}

/// The second referenced value group in `ValueFlow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueFlowFees {
    /// Imported fees.
    pub fees_imported: CurrencyCollection,
    /// Recovered value.
    pub recovered: CurrencyCollection,
    /// Created value.
    pub created: CurrencyCollection,
    /// Minted value.
    pub minted: CurrencyCollection,
}

impl TlbSerialize for ValueFlowFees {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        self.fees_imported.store_tlb(builder)?;
        self.recovered.store_tlb(builder)?;
        self.created.store_tlb(builder)?;
        self.minted.store_tlb(builder)
    }
}

impl TlbDeserialize for ValueFlowFees {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            fees_imported: CurrencyCollection::load_tlb(slice)?,
            recovered: CurrencyCollection::load_tlb(slice)?,
            created: CurrencyCollection::load_tlb(slice)?,
            minted: CurrencyCollection::load_tlb(slice)?,
        })
    }
}

/// TL-B `ValueFlow`, with both known constructor layouts represented directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueFlow {
    /// `value_flow#b8e48dfb`.
    V1 {
        /// Referenced source values.
        main: ValueFlowMain,
        /// Fees collected inline.
        fees_collected: CurrencyCollection,
        /// Referenced fee values.
        fees: ValueFlowFees,
    },
    /// `value_flow_v2#3ebf98b7`.
    V2 {
        /// Referenced source values.
        main: ValueFlowMain,
        /// Fees collected inline.
        fees_collected: CurrencyCollection,
        /// Burned value.
        burned: CurrencyCollection,
        /// Referenced fee values.
        fees: ValueFlowFees,
    },
}

impl TlbSerialize for ValueFlow {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        match self {
            Self::V1 {
                main,
                fees_collected,
                fees,
            } => {
                builder.store_u32(VALUE_FLOW_TAG)?;
                store_ref_tlb(builder, main)?;
                fees_collected.store_tlb(builder)?;
                store_ref_tlb(builder, fees)?;
            }
            Self::V2 {
                main,
                fees_collected,
                burned,
                fees,
            } => {
                builder.store_u32(VALUE_FLOW_V2_TAG)?;
                store_ref_tlb(builder, main)?;
                fees_collected.store_tlb(builder)?;
                burned.store_tlb(builder)?;
                store_ref_tlb(builder, fees)?;
            }
        }
        Ok(())
    }
}

impl TlbDeserialize for ValueFlow {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let tag = slice.load_u32()?;
        match tag {
            VALUE_FLOW_TAG => {
                let main = load_ref_tlb(slice, "ValueFlowMain")?;
                let fees_collected = CurrencyCollection::load_tlb(slice)?;
                Ok(Self::V1 {
                    main,
                    fees_collected,
                    fees: load_ref_tlb(slice, "ValueFlowFees")?,
                })
            }
            VALUE_FLOW_V2_TAG => {
                let main = load_ref_tlb(slice, "ValueFlowMain")?;
                let fees_collected = CurrencyCollection::load_tlb(slice)?;
                Ok(Self::V2 {
                    main,
                    fees_collected,
                    burned: CurrencyCollection::load_tlb(slice)?,
                    fees: load_ref_tlb(slice, "ValueFlowFees")?,
                })
            }
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

/// TL-B `ShardStateUnsplit` payload preserved as a raw cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardStateUnsplit {
    /// Original unsplit shard-state cell.
    pub cell: Arc<Cell>,
}

/// Typed fields of `shard_state#9023afe2`.
///
/// `OutMsgQueueInfo`, `LibDescr` and `McStateExtra` do not have typed models
/// yet, so their cell boundaries are preserved explicitly rather than being
/// folded into one opaque shard-state payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardStateUnsplitData {
    /// Global network id.
    pub global_id: i32,
    /// Shard identifier.
    pub shard_id: ShardIdent,
    /// Horizontal sequence number.
    pub seq_no: u32,
    /// Vertical sequence number.
    pub vert_seq_no: u32,
    /// Generation Unix timestamp.
    pub gen_utime: u32,
    /// Generation logical time.
    pub gen_lt: u64,
    /// Minimum referenced masterchain sequence number.
    pub min_ref_mc_seqno: u32,
    /// Raw `OutMsgQueueInfo` reference until that family is typed.
    pub out_msg_queue_info: Arc<Cell>,
    /// Whether the shard state is before a split.
    pub before_split: bool,
    /// Typed shard-account dictionary.
    pub accounts: ShardAccounts,
    /// Overload history counter.
    pub overload_history: u64,
    /// Underload history counter.
    pub underload_history: u64,
    /// Total shard balance.
    pub total_balance: CurrencyCollection,
    /// Total validator fees.
    pub total_validator_fees: CurrencyCollection,
    /// Raw `HashmapE 256 LibDescr` reference until `LibDescr` is typed.
    pub libraries: Arc<Cell>,
    /// Optional masterchain reference.
    pub master_ref: Option<BlkMasterInfo>,
    /// Raw optional `McStateExtra` reference until that family is typed.
    pub custom: Option<Arc<Cell>>,
}

impl ShardStateUnsplit {
    /// Decodes the stable fields of the shard state while preserving
    /// unsupported nested families at their schema-defined cell boundaries.
    pub fn decode_typed(&self) -> Result<ShardStateUnsplitData> {
        ShardStateUnsplitData::from_cell(self.cell.clone())
    }
}

impl TlbSerialize for ShardStateUnsplitData {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u32(SHARD_STATE_TAG)?;
        builder.store_int(self.global_id as i64, 32)?;
        self.shard_id.store_tlb(builder)?;
        builder.store_u32(self.seq_no)?;
        builder.store_u32(self.vert_seq_no)?;
        builder.store_u32(self.gen_utime)?;
        builder.store_u64(self.gen_lt)?;
        builder.store_u32(self.min_ref_mc_seqno)?;
        builder.store_ref(self.out_msg_queue_info.clone())?;
        builder.store_bit(self.before_split)?;
        store_ref_tlb(builder, &self.accounts)?;

        let extra = ShardStateUnsplitExtra {
            overload_history: self.overload_history,
            underload_history: self.underload_history,
            total_balance: self.total_balance.clone(),
            total_validator_fees: self.total_validator_fees.clone(),
            libraries: self.libraries.clone(),
            master_ref: self.master_ref.clone(),
        };
        store_ref_tlb(builder, &extra)?;
        builder.store_bit(self.custom.is_some())?;
        if let Some(custom) = &self.custom {
            builder.store_ref(custom.clone())?;
        }
        Ok(())
    }
}

impl TlbDeserialize for ShardStateUnsplitData {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let tag = slice.load_u32()?;
        if tag != SHARD_STATE_TAG {
            return Err(TlbError::TagMismatch {
                constructor: "ShardStateUnsplit",
                expected_bits: "9023afe2",
                actual_bits: format!("{tag:08x}"),
            });
        }
        let global_id = slice.load_int(32)? as i32;
        let shard_id = ShardIdent::load_tlb(slice)?;
        let seq_no = slice.load_u32()?;
        let vert_seq_no = slice.load_u32()?;
        let gen_utime = slice.load_u32()?;
        let gen_lt = slice.load_u64()?;
        let min_ref_mc_seqno = slice.load_u32()?;
        let out_msg_queue_info = slice.load_reference()?;
        let before_split = slice.load_bit()?;
        let accounts = load_ref_tlb(slice, "ShardAccounts")?;
        let extra: ShardStateUnsplitExtra = load_ref_tlb(slice, "ShardStateUnsplitExtra")?;
        let custom = if slice.load_bit()? {
            Some(slice.load_reference()?)
        } else {
            None
        };
        Ok(Self {
            global_id,
            shard_id,
            seq_no,
            vert_seq_no,
            gen_utime,
            gen_lt,
            min_ref_mc_seqno,
            out_msg_queue_info,
            before_split,
            accounts,
            overload_history: extra.overload_history,
            underload_history: extra.underload_history,
            total_balance: extra.total_balance,
            total_validator_fees: extra.total_validator_fees,
            libraries: extra.libraries,
            master_ref: extra.master_ref,
            custom,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShardStateUnsplitExtra {
    overload_history: u64,
    underload_history: u64,
    total_balance: CurrencyCollection,
    total_validator_fees: CurrencyCollection,
    libraries: Arc<Cell>,
    master_ref: Option<BlkMasterInfo>,
}

impl TlbSerialize for ShardStateUnsplitExtra {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_u64(self.overload_history)?;
        builder.store_u64(self.underload_history)?;
        self.total_balance.store_tlb(builder)?;
        self.total_validator_fees.store_tlb(builder)?;
        builder.store_ref(self.libraries.clone())?;
        builder.store_bit(self.master_ref.is_some())?;
        if let Some(master_ref) = &self.master_ref {
            master_ref.store_tlb(builder)?;
        }
        Ok(())
    }
}

impl TlbDeserialize for ShardStateUnsplitExtra {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            overload_history: slice.load_u64()?,
            underload_history: slice.load_u64()?,
            total_balance: CurrencyCollection::load_tlb(slice)?,
            total_validator_fees: CurrencyCollection::load_tlb(slice)?,
            libraries: slice.load_reference()?,
            master_ref: if slice.load_bit()? {
                Some(BlkMasterInfo::load_tlb(slice)?)
            } else {
                None
            },
        })
    }
}

impl TlbSerialize for ShardStateUnsplit {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.cell)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardStateUnsplit {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        let state = ShardState::load_tlb(slice)?;
        match state {
            ShardState::Unsplit { payload } => Ok(Self { cell: payload }),
            ShardState::Split { .. } => Err(TlbError::TagMismatch {
                constructor: "ShardStateUnsplit",
                expected_bits: "9023afe2",
                actual_bits: "5f327da5".to_string(),
            }),
        }
    }
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

/// Raw-preserving typed view over a config parameter dictionary entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigParam {
    /// Config parameter id.
    pub id: u32,
    /// Typed family currently recognized by this crate.
    pub value: ConfigParamValue,
}

/// Config parameter families needed by block/config/proof-adjacent APIs.
///
/// Exact deep schemas remain intentionally raw-preserving until fixture-backed
/// upstream evidence is checked in for each family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigParamValue {
    /// Config parameter 0.
    Param0(Arc<Cell>),
    /// Config parameter 1.
    Param1(Arc<Cell>),
    /// Config parameter 2.
    Param2(Arc<Cell>),
    /// Config parameter 15.
    Param15(Arc<Cell>),
    /// Config parameter 17.
    Param17(Arc<Cell>),
    /// Config parameter 18.
    Param18(Arc<Cell>),
    /// Config parameter 20.
    Param20(Arc<Cell>),
    /// Config parameter 21.
    Param21(Arc<Cell>),
    /// Config parameter 24.
    Param24(Arc<Cell>),
    /// Config parameter 25.
    Param25(Arc<Cell>),
    /// Config parameter 32.
    Param32(Arc<Cell>),
    /// Config parameter 34.
    Param34(Arc<Cell>),
    /// Config parameter 36.
    Param36(Arc<Cell>),
    /// Unknown config parameter preserved as raw cell.
    Unknown { id: u32, raw: Arc<Cell> },
}

/// TL-B `update_hashes#72 old_hash:bits256 new_hash:bits256 = HASH_UPDATE X`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashUpdate {
    /// Old representation hash.
    pub old_hash: [u8; 32],
    /// New representation hash.
    pub new_hash: [u8; 32],
}

impl TlbSerialize for HashUpdate {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        store_tag(builder, "01110010")?;
        builder.store_bytes(&self.old_hash)?;
        builder.store_bytes(&self.new_hash)?;
        Ok(())
    }
}

impl TlbDeserialize for HashUpdate {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag(slice, "HASH_UPDATE", "01110010")?;
        Ok(Self {
            old_hash: load_hash(slice)?,
            new_hash: load_hash(slice)?,
        })
    }
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

impl ConfigParams {
    /// Decodes `config:^(Hashmap 32 ^Cell)` while preserving each parameter
    /// cell unchanged.
    pub fn config_entries(&self) -> Result<HashmapE<Arc<Cell>>> {
        let mut slice = Slice::new(self.config.clone());
        slice
            .load_hashmap_e_with(CONFIG_PARAMS_KEY_BITS, |slice| slice.load_reference())
            .map_err(|error| TlbError::CustomSchema {
                schema: "ConfigParams.config",
                message: error.to_string(),
            })
    }

    /// Returns raw-preserving typed wrappers for known config parameter ids.
    pub fn typed_params(&self) -> Result<Vec<ConfigParam>> {
        self.config_entries()?
            .iter()
            .map(|(key, raw)| {
                let id = key.to_u64().map_err(|error| TlbError::CustomSchema {
                    schema: "ConfigParams.config.key",
                    message: error.to_string(),
                })? as u32;
                Ok(ConfigParam {
                    id,
                    value: ConfigParamValue::from_raw(id, raw.clone()),
                })
            })
            .collect()
    }

    /// Looks up one raw config parameter by id.
    pub fn raw_param(&self, id: u32) -> Result<Option<Arc<Cell>>> {
        let key = BitKey::from_u64(id as u64, CONFIG_PARAMS_KEY_BITS).map_err(|error| {
            TlbError::CustomSchema {
                schema: "ConfigParams.config.key",
                message: error.to_string(),
            }
        })?;
        self.config_entries()?
            .get_bit_key(&key)
            .map(|value| value.cloned())
            .map_err(|error| TlbError::CustomSchema {
                schema: "ConfigParams.config",
                message: error.to_string(),
            })
    }
}

impl ConfigParamValue {
    fn from_raw(id: u32, raw: Arc<Cell>) -> Self {
        match id {
            0 => Self::Param0(raw),
            1 => Self::Param1(raw),
            2 => Self::Param2(raw),
            15 => Self::Param15(raw),
            17 => Self::Param17(raw),
            18 => Self::Param18(raw),
            20 => Self::Param20(raw),
            21 => Self::Param21(raw),
            24 => Self::Param24(raw),
            25 => Self::Param25(raw),
            32 => Self::Param32(raw),
            34 => Self::Param34(raw),
            36 => Self::Param36(raw),
            _ => Self::Unknown { id, raw },
        }
    }
}

fn load_hash(slice: &mut Slice) -> Result<[u8; 32]> {
    let mut hash = [0; 32];
    hash.copy_from_slice(&slice.load_bytes(32)?);
    Ok(hash)
}

fn store_maybe_ref<T: TlbSerialize>(builder: &mut Builder, value: &Option<T>) -> Result<()> {
    builder.store_bit(value.is_some())?;
    if let Some(value) = value {
        builder.store_ref(value.to_cell()?)?;
    }
    Ok(())
}

fn load_maybe_ref<T: TlbDeserialize>(slice: &mut Slice, schema: &'static str) -> Result<Option<T>> {
    if !slice.load_bit()? {
        return Ok(None);
    }
    Ok(Some(load_ref_tlb(slice, schema)?))
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
#[path = "block_tests.rs"]
mod tests;
