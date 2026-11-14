//! NFT contract data models (TEP-62 / TEP-85).
//!
//! Collection and item editions use different storage layouts; each has its own
//! type here. Content cells follow TEP-64: collection content carries the prefix
//! byte, while item and common content are bare snake strings.

use std::sync::Arc;

use crate::{
    contracts::{load_address_field, store_address_field},
    metadata::{Tep64Content, parse_tep64_content, parse_tep64_offchain},
    tlb::{TlbDeserialize, TlbError, TlbResult, TlbSerialize},
    tvm::{Address, Builder, Cell, Slice},
};

/// Royalty parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoyaltyParams {
    /// Royalty numerator.
    pub numerator: u16,
    /// Royalty denominator.
    pub denominator: u16,
    /// Royalty recipient.
    pub address: Option<Address>,
}

impl RoyaltyParams {
    /// Creates royalty parameters.
    pub fn new(numerator: u16, denominator: u16, address: Option<Address>) -> Self {
        Self {
            numerator,
            denominator,
            address,
        }
    }

    /// Returns the royalty fraction as a floating point value.
    pub fn fraction(&self) -> f64 {
        if self.denominator == 0 {
            0.0
        } else {
            f64::from(self.numerator) / f64::from(self.denominator)
        }
    }
}

impl TlbSerialize for RoyaltyParams {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u16(self.numerator)?;
        builder.store_u16(self.denominator)?;
        store_address_field(builder, self.address.as_ref())?;
        Ok(())
    }
}

impl TlbDeserialize for RoyaltyParams {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            numerator: slice.load_u16()?,
            denominator: slice.load_u16()?,
            address: load_address_field(slice)?,
        })
    }
}

/// Collection metadata root: collection content plus common item-content prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTCollectionContent {
    /// Collection metadata with the TEP-64 prefix.
    pub content: Tep64Content,
    /// Common base URI for item metadata, stored as a bare snake string.
    pub common_content: Tep64Content,
}

impl NFTCollectionContent {
    /// Creates collection content.
    pub fn new(content: Tep64Content, common_content: Tep64Content) -> Self {
        Self {
            content,
            common_content,
        }
    }

    /// Encodes the content root cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        let content = content_cell(&self.content, true)?;
        let common = content_cell(&self.common_content, false)?;
        let mut builder = Builder::new();
        builder.store_ref(content)?;
        builder.store_ref(common)?;
        Ok(builder.build()?)
    }

    /// Decodes the content root cell.
    pub fn from_cell(cell: &Arc<Cell>) -> TlbResult<Self> {
        let mut slice = Slice::new(cell.clone());
        let content = parse_content(slice.load_reference()?, true)?;
        let common_content = parse_content(slice.load_reference()?, false)?;
        if !slice.is_empty() {
            return Err(TlbError::TrailingData {
                bits: slice.remaining_bits(),
                refs: slice.remaining_refs(),
            });
        }
        Ok(Self {
            content,
            common_content,
        })
    }
}

/// NFT collection storage shared by standard and editable collections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTCollectionData {
    /// Collection owner.
    pub owner_address: Option<Address>,
    /// Index that will be assigned to the next minted item.
    pub next_item_index: u64,
    /// Collection metadata.
    pub content: NFTCollectionContent,
    /// Code cell used by items of this collection.
    pub nft_item_code: Arc<Cell>,
    /// Royalty parameters.
    pub royalty_params: RoyaltyParams,
}

impl TlbSerialize for NFTCollectionData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_u64(self.next_item_index)?;
        builder.store_ref(self.content.to_cell()?)?;
        builder.store_ref(self.nft_item_code.clone())?;
        builder.store_ref(self.royalty_params.to_cell()?)?;
        Ok(())
    }
}

impl TlbDeserialize for NFTCollectionData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            owner_address: load_address_field(slice)?,
            next_item_index: slice.load_u64()?,
            content: NFTCollectionContent::from_cell(&slice.load_reference()?)?,
            nft_item_code: slice.load_reference()?,
            royalty_params: RoyaltyParams::load_tlb(&mut Slice::new(slice.load_reference()?))?,
        })
    }
}

/// Standard NFT item storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemStandardData {
    /// Item index in the collection.
    pub index: u64,
    /// Parent collection.
    pub collection_address: Option<Address>,
    /// Current owner.
    pub owner_address: Option<Address>,
    /// Item metadata, stored as a bare snake string.
    pub content: Tep64Content,
}

impl TlbSerialize for NFTItemStandardData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u64(self.index)?;
        store_address_field(builder, self.collection_address.as_ref())?;
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        Ok(())
    }
}

impl TlbDeserialize for NFTItemStandardData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            index: slice.load_u64()?,
            collection_address: load_address_field(slice)?,
            owner_address: load_address_field(slice)?,
            content: parse_content(slice.load_reference()?, false)?,
        })
    }
}

/// Editable NFT item storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemEditableData {
    /// Item index in the collection.
    pub index: u64,
    /// Parent collection.
    pub collection_address: Option<Address>,
    /// Current owner.
    pub owner_address: Option<Address>,
    /// Item metadata, stored as a bare snake string.
    pub content: Tep64Content,
    /// Address authorized to edit content.
    pub editor_address: Option<Address>,
}

impl TlbSerialize for NFTItemEditableData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u64(self.index)?;
        store_address_field(builder, self.collection_address.as_ref())?;
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        store_address_field(builder, self.editor_address.as_ref())?;
        Ok(())
    }
}

impl TlbDeserialize for NFTItemEditableData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            index: slice.load_u64()?,
            collection_address: load_address_field(slice)?,
            owner_address: load_address_field(slice)?,
            content: parse_content(slice.load_reference()?, false)?,
            editor_address: load_address_field(slice)?,
        })
    }
}

/// Soulbound NFT item storage (TEP-85).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemSoulboundData {
    /// Item index in the collection.
    pub index: u64,
    /// Parent collection.
    pub collection_address: Option<Address>,
    /// Current owner.
    pub owner_address: Option<Address>,
    /// Item metadata, stored as a bare snake string.
    pub content: Tep64Content,
    /// Authority allowed to revoke the token.
    pub authority_address: Option<Address>,
    /// Revocation unix timestamp, or zero.
    pub revoked_at: u64,
}

impl TlbSerialize for NFTItemSoulboundData {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u64(self.index)?;
        store_address_field(builder, self.collection_address.as_ref())?;
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        store_address_field(builder, self.authority_address.as_ref())?;
        builder.store_u64(self.revoked_at)?;
        Ok(())
    }
}

impl TlbDeserialize for NFTItemSoulboundData {
    fn load_tlb(slice: &mut Slice) -> TlbResult<Self> {
        Ok(Self {
            index: slice.load_u64()?,
            collection_address: load_address_field(slice)?,
            owner_address: load_address_field(slice)?,
            content: parse_content(slice.load_reference()?, false)?,
            authority_address: load_address_field(slice)?,
            revoked_at: slice.load_u64()?,
        })
    }
}

/// Standard item mint reference (`owner`, `content`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemStandardMintRef {
    /// Initial owner.
    pub owner_address: Option<Address>,
    /// Item metadata.
    pub content: Tep64Content,
}

impl TlbSerialize for NFTItemStandardMintRef {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        Ok(())
    }
}

/// Editable item mint reference (`owner`, `editor`, `content`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemEditableMintRef {
    /// Initial owner.
    pub owner_address: Option<Address>,
    /// Initial editor.
    pub editor_address: Option<Address>,
    /// Item metadata.
    pub content: Tep64Content,
}

impl TlbSerialize for NFTItemEditableMintRef {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        store_address_field(builder, self.owner_address.as_ref())?;
        store_address_field(builder, self.editor_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        Ok(())
    }
}

/// Soulbound item mint reference (`owner`, `content`, `authority`, `revoked_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NFTItemSoulboundMintRef {
    /// Initial owner.
    pub owner_address: Option<Address>,
    /// Item metadata.
    pub content: Tep64Content,
    /// Authority allowed to revoke the token.
    pub authority_address: Option<Address>,
    /// Initial revocation timestamp.
    pub revoked_at: u64,
}

impl TlbSerialize for NFTItemSoulboundMintRef {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        store_address_field(builder, self.owner_address.as_ref())?;
        builder.store_ref(content_cell(&self.content, false)?)?;
        store_address_field(builder, self.authority_address.as_ref())?;
        builder.store_u64(self.revoked_at)?;
        Ok(())
    }
}

fn content_cell(content: &Tep64Content, with_prefix: bool) -> TlbResult<Arc<Cell>> {
    content
        .to_cell_with_prefix(with_prefix)
        .map_err(|error| TlbError::CustomSchema {
            schema: "Tep64Content",
            message: error.to_string(),
        })
}

fn parse_content(cell: Arc<Cell>, with_prefix: bool) -> TlbResult<Tep64Content> {
    let parsed = if with_prefix {
        parse_tep64_content(cell)
    } else {
        parse_tep64_offchain(cell)
    };
    parsed.map_err(|error| TlbError::CustomSchema {
        schema: "Tep64Content",
        message: error.to_string(),
    })
}
