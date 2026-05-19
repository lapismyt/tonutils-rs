//! Generated-backed TL-B codecs for Phase 1 block, config, and proof models.
//!
//! These types cover the cell boundaries and constructor tags needed by
//! LiteClient BoC decoding and offline proof primitive tests. Deep block
//! families that are still generated as raw child cells preserve their exact
//! bytes and references so callers can inspect hashes before opting into
//! verification.

use crate::tlb::{Result, TlbDeserialize, TlbError, TlbSerialize, expect_tag, store_tag};
use crate::tvm::{BitKey, Builder, Cell, HashmapE, Slice};
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
        builder.store_uint_custom::<u8>(self.shard_pfx_bits as u8, 6)?;
        builder.store_int(self.workchain_id as i64, 32)?;
        builder.store_u64(self.shard_prefix)?;
        Ok(())
    }
}

impl TlbDeserialize for ShardIdent {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        expect_tag(slice, "ShardIdent", "00")?;
        let shard_pfx_bits = slice.load_uint_custom::<u8>(6)? as u8;
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

/// TL-B `BlockInfo` payload preserved as a raw cell until deep block-info
/// fields are generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockInfo {
    /// Original `BlockInfo` cell.
    pub cell: Arc<Cell>,
}

impl TlbSerialize for BlockInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.cell)?;
        Ok(())
    }
}

impl TlbDeserialize for BlockInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            cell: consume_remaining_cell(slice)?,
        })
    }
}

/// TL-B `BlockPrevInfo` payload preserved as a raw cell until the predecessor
/// union is generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPrevInfo {
    /// Original `BlockPrevInfo` cell.
    pub cell: Arc<Cell>,
}

impl TlbSerialize for BlockPrevInfo {
    fn store_tlb(&self, builder: &mut Builder) -> Result<()> {
        builder.store_cell(&self.cell)?;
        Ok(())
    }
}

impl TlbDeserialize for BlockPrevInfo {
    fn load_tlb(slice: &mut Slice) -> Result<Self> {
        Ok(Self {
            cell: consume_remaining_cell(slice)?,
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

/// TL-B `BlockExtra`, preserved as raw children while generated coverage grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExtra {
    /// Original cell.
    pub cell: Arc<Cell>,
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

/// TL-B `ShardStateUnsplit` payload preserved as a raw cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardStateUnsplit {
    /// Original unsplit shard-state cell.
    pub cell: Arc<Cell>,
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

    #[test]
    fn hash_update_uses_eight_bit_constructor_tag() {
        let update = HashUpdate {
            old_hash: [0x11; 32],
            new_hash: [0x22; 32],
        };

        let cell = update.to_cell().unwrap();
        assert_eq!(cell.bit_len(), 8 + 256 + 256);
        assert_eq!(HashUpdate::from_cell(cell).unwrap(), update);
    }
}
