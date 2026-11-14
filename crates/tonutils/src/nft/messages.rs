//! NFT message bodies (TEP-62 / TEP-85).

use std::sync::Arc;

use crate::{
    contracts::{ForwardPayload, store_address_field},
    metadata::Tep64Content,
    tlb::{TlbError, TlbResult, TlbSerialize},
    tvm::{Address, Builder, Cell, HashmapE},
};

use super::data::{NFTCollectionContent, RoyaltyParams};

/// `transfer#5fcc3d14` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftTransfer {
    /// Query identifier.
    pub query_id: u64,
    /// New owner.
    pub new_owner: Option<Address>,
    /// Address that receives excess TON.
    pub response_destination: Option<Address>,
    /// Optional custom payload.
    pub custom_payload: Option<Arc<Cell>>,
    /// TON amount forwarded with the ownership notification.
    pub forward_ton_amount: u128,
    /// Payload delivered to the new owner.
    pub forward_payload: ForwardPayload,
}

impl NftTransfer {
    /// Creates a transfer body.
    pub fn new(new_owner: Address) -> Self {
        Self {
            query_id: 0,
            new_owner: Some(new_owner),
            response_destination: None,
            custom_payload: None,
            forward_ton_amount: 0,
            forward_payload: ForwardPayload::None,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftTransfer {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x5fcc_3d14)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, self.new_owner.as_ref())?;
        store_address_field(builder, self.response_destination.as_ref())?;
        builder.store_maybe_ref(self.custom_payload.clone())?;
        builder.store_coins(self.forward_ton_amount)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

/// `mint#00000001` collection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftCollectionMintItem {
    /// Query identifier.
    pub query_id: u64,
    /// Index assigned to the new item.
    pub item_index: u64,
    /// TON amount forwarded to the new item.
    pub forward_ton_amount: u128,
    /// Item mint reference cell.
    pub item_ref: Arc<Cell>,
}

impl NftCollectionMintItem {
    /// Creates a mint body.
    pub fn new(item_index: u64, forward_ton_amount: u128, item_ref: Arc<Cell>) -> Self {
        Self {
            query_id: 0,
            item_index,
            forward_ton_amount,
            item_ref,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftCollectionMintItem {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(1)?;
        builder.store_u64(self.query_id)?;
        builder.store_u64(self.item_index)?;
        builder.store_coins(self.forward_ton_amount)?;
        builder.store_ref(self.item_ref.clone())?;
        Ok(())
    }
}

/// `batch_mint#00000002` collection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftCollectionBatchMint {
    /// Query identifier.
    pub query_id: u64,
    /// First item index in the batch.
    pub from_index: u64,
    /// TON amount forwarded to each item.
    pub forward_ton_amount: u128,
    /// `(index, amount, item_ref)` tuples. `amount` is the TON amount attached
    /// to that individual mint message.
    pub items: Vec<(u64, u128, Arc<Cell>)>,
}

impl NftCollectionBatchMint {
    /// Maximum number of items accepted by the reference contract.
    pub const MAX_ITEMS: usize = 250;

    /// Creates a batch mint body.
    pub fn new(
        from_index: u64,
        forward_ton_amount: u128,
        items: Vec<(u64, u128, Arc<Cell>)>,
    ) -> TlbResult<Self> {
        if items.len() > Self::MAX_ITEMS {
            return Err(TlbError::CustomSchema {
                schema: "NftCollectionBatchMint",
                message: format!(
                    "batch size {} exceeds maximum {}",
                    items.len(),
                    Self::MAX_ITEMS
                ),
            });
        }
        Ok(Self {
            query_id: 0,
            from_index,
            forward_ton_amount,
            items,
        })
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftCollectionBatchMint {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        let mut dict: HashmapE<(u128, Arc<Cell>)> = HashmapE::new(64);
        for (index, amount, item_ref) in &self.items {
            let key = crate::tvm::BitKey::from_u64(*index, 64)?;
            dict.insert_bit_key(key, (*amount, item_ref.clone()))?;
        }
        builder.store_u32(2)?;
        builder.store_u64(self.query_id)?;
        builder.store_u64(self.from_index)?;
        builder.store_coins(self.forward_ton_amount)?;
        builder.store_hashmap_e_with(&dict, |builder, (amount, item_ref)| {
            builder.store_coins(*amount)?;
            builder.store_ref(item_ref.clone())?;
            Ok(())
        })?;
        Ok(())
    }
}

/// `change_owner#00000003` collection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftCollectionChangeOwner {
    /// Query identifier.
    pub query_id: u64,
    /// New owner.
    pub owner_address: Option<Address>,
}

impl NftCollectionChangeOwner {
    /// Creates a change-owner body.
    pub fn new(owner_address: Option<Address>) -> Self {
        Self {
            query_id: 0,
            owner_address,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftCollectionChangeOwner {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(3)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, self.owner_address.as_ref())?;
        Ok(())
    }
}

/// `change_content#00000004` collection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftCollectionChangeContent {
    /// Query identifier.
    pub query_id: u64,
    /// New collection content.
    pub content: NFTCollectionContent,
    /// New royalty parameters.
    pub royalty_params: RoyaltyParams,
}

impl NftCollectionChangeContent {
    /// Creates a change-content body.
    pub fn new(content: NFTCollectionContent, royalty_params: RoyaltyParams) -> Self {
        Self {
            query_id: 0,
            content,
            royalty_params,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftCollectionChangeContent {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(4)?;
        builder.store_u64(self.query_id)?;
        builder.store_ref(self.content.to_cell()?)?;
        builder.store_ref(self.royalty_params.to_cell()?)?;
        Ok(())
    }
}

/// `edit_content#1a0b9d51` item body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftEditContent {
    /// Query identifier.
    pub query_id: u64,
    /// New item content.
    pub content: Tep64Content,
}

impl NftEditContent {
    /// Creates an edit-content body.
    pub fn new(content: Tep64Content) -> Self {
        Self {
            query_id: 0,
            content,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftEditContent {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        let content =
            self.content
                .to_cell_with_prefix(false)
                .map_err(|error| TlbError::CustomSchema {
                    schema: "NftEditContent",
                    message: error.to_string(),
                })?;
        builder.store_u32(0x1a0b_9d51)?;
        builder.store_u64(self.query_id)?;
        builder.store_ref(content)?;
        Ok(())
    }
}

/// `transfer_editorship#1c04412a` item body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftTransferEditorship {
    /// Query identifier.
    pub query_id: u64,
    /// New editor.
    pub editor_address: Option<Address>,
    /// Address that receives excess TON.
    pub response_destination: Option<Address>,
    /// Optional custom payload.
    pub custom_payload: Option<Arc<Cell>>,
    /// TON amount forwarded with the editorship notification.
    pub forward_ton_amount: u128,
    /// Payload delivered to the new editor.
    pub forward_payload: ForwardPayload,
}

impl NftTransferEditorship {
    /// Creates a transfer-editorship body.
    pub fn new(editor_address: Option<Address>) -> Self {
        Self {
            query_id: 0,
            editor_address,
            response_destination: None,
            custom_payload: None,
            forward_ton_amount: 0,
            forward_payload: ForwardPayload::None,
        }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl TlbSerialize for NftTransferEditorship {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x1c04_412a)?;
        builder.store_u64(self.query_id)?;
        store_address_field(builder, self.editor_address.as_ref())?;
        store_address_field(builder, self.response_destination.as_ref())?;
        builder.store_maybe_ref(self.custom_payload.clone())?;
        builder.store_coins(self.forward_ton_amount)?;
        self.forward_payload.store_tlb(builder)?;
        Ok(())
    }
}

/// `destroy#1f04537a` soulbound body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftDestroy {
    /// Query identifier.
    pub query_id: u64,
}

impl NftDestroy {
    /// Creates a destroy body.
    pub fn new() -> Self {
        Self { query_id: 0 }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl Default for NftDestroy {
    fn default() -> Self {
        Self::new()
    }
}

impl TlbSerialize for NftDestroy {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x1f04_537a)?;
        builder.store_u64(self.query_id)?;
        Ok(())
    }
}

/// `revoke#6f89f5e3` soulbound body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftRevoke {
    /// Query identifier.
    pub query_id: u64,
}

impl NftRevoke {
    /// Creates a revoke body.
    pub fn new() -> Self {
        Self { query_id: 0 }
    }

    /// Serializes the body into a cell.
    pub fn to_cell(&self) -> TlbResult<Arc<Cell>> {
        TlbSerialize::to_cell(self)
    }
}

impl Default for NftRevoke {
    fn default() -> Self {
        Self::new()
    }
}

impl TlbSerialize for NftRevoke {
    fn store_tlb(&self, builder: &mut Builder) -> TlbResult<()> {
        builder.store_u32(0x6f89_f5e3)?;
        builder.store_u64(self.query_id)?;
        Ok(())
    }
}
