//! NFT collection wrappers.

use std::sync::Arc;

use crate::{
    contracts::{
        ContractBuildError, ContractCodeError, ContractDefinition, ContractSpec, ContractVersion,
        ForwardPayload, store_address_field,
    },
    tlb::{StateInit, TlbSerialize},
    tvm::{Address, Builder, Cell},
};

use super::data::{NFTCollectionContent, NFTCollectionData, RoyaltyParams};

/// Standard NFT collection (TEP-62).
#[derive(Debug, Clone)]
pub struct NFTCollectionStandard {
    spec: ContractSpec<NFTCollectionData>,
}

impl NFTCollectionStandard {
    /// Creates a collection with embedded default collection and item code.
    pub fn new(
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
    ) -> Result<Self, ContractCodeError> {
        let item_code = ContractVersion::NFTItemStandard.default_code()?;
        Self::with_item_code(owner_address, content, royalty_params, item_code)
    }

    /// Creates a collection with a custom item code.
    pub fn with_item_code(
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
        nft_item_code: Arc<Cell>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::NFTCollectionStandard,
                NFTCollectionData {
                    owner_address,
                    next_item_index: 0,
                    content,
                    nft_item_code,
                    royalty_params,
                },
            )?,
        })
    }

    /// Creates a collection with explicit collection code.
    pub fn with_code(
        code: Arc<Cell>,
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
        nft_item_code: Arc<Cell>,
    ) -> Self {
        Self::from_parts(
            code,
            NFTCollectionData {
                owner_address,
                next_item_index: 0,
                content,
                nft_item_code,
                royalty_params,
            },
        )
    }

    /// Creates a collection from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: NFTCollectionData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a collection from decoded data and embedded default code.
    pub fn from_data(data: NFTCollectionData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::NFTCollectionStandard, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Collection owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Next item index.
    pub fn next_item_index(&self) -> u64 {
        self.spec.data.next_item_index
    }

    /// Collection metadata.
    pub fn content(&self) -> &NFTCollectionContent {
        &self.spec.data.content
    }

    /// Item code published by the collection.
    pub fn nft_item_code(&self) -> &Arc<Cell> {
        &self.spec.data.nft_item_code
    }

    /// Royalty parameters.
    pub fn royalty_params(&self) -> &RoyaltyParams {
        &self.spec.data.royalty_params
    }

    /// Computes an item address using this collection's code and address.
    pub fn item_address(&self, index: u64) -> Result<Address, ContractBuildError> {
        let collection = self.address()?;
        calculate_item_address(
            index,
            &collection,
            &self.spec.data.nft_item_code,
            self.spec.workchain,
        )
    }

    /// Builds a single-item mint body with a standard item mint reference.
    pub fn mint_body(
        &self,
        owner_address: Option<Address>,
        content: crate::metadata::Tep64Content,
        item_index: u64,
        forward_ton_amount: u128,
    ) -> Result<super::messages::NftCollectionMintItem, ContractBuildError> {
        let item_ref = super::data::NFTItemStandardMintRef {
            owner_address,
            content,
        }
        .to_cell()
        .map_err(ContractBuildError::DataSerialization)?;
        Ok(super::messages::NftCollectionMintItem::new(
            item_index,
            forward_ton_amount,
            item_ref,
        ))
    }

    /// Builds a transfer body for an item owned by this collection.
    pub fn transfer_body(new_owner: Address) -> super::messages::NftTransfer {
        super::messages::NftTransfer::new(new_owner)
    }

    /// Builds a transfer body with a forward payload.
    pub fn transfer_body_with_payload(
        new_owner: Address,
        payload: ForwardPayload,
    ) -> super::messages::NftTransfer {
        let mut body = super::messages::NftTransfer::new(new_owner);
        body.forward_payload = payload;
        body
    }
}

impl ContractDefinition for NFTCollectionStandard {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::NFTCollectionStandard.default_code()
    }
    type Data = NFTCollectionData;

    fn code(&self) -> &Arc<Cell> {
        &self.spec.code
    }

    fn data(&self) -> Self::Data {
        self.spec.data.clone()
    }

    fn workchain(&self) -> i8 {
        self.spec.workchain
    }
}

/// Editable NFT collection (TEP-62).
#[derive(Debug, Clone)]
pub struct NFTCollectionEditable {
    spec: ContractSpec<NFTCollectionData>,
}

impl NFTCollectionEditable {
    /// Creates a collection with embedded default collection and item code.
    pub fn new(
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
    ) -> Result<Self, ContractCodeError> {
        let item_code = ContractVersion::NFTItemEditable.default_code()?;
        Self::with_item_code(owner_address, content, royalty_params, item_code)
    }

    /// Creates a collection with a custom item code.
    pub fn with_item_code(
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
        nft_item_code: Arc<Cell>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::NFTCollectionEditable,
                NFTCollectionData {
                    owner_address,
                    next_item_index: 0,
                    content,
                    nft_item_code,
                    royalty_params,
                },
            )?,
        })
    }

    /// Creates a collection with explicit collection code.
    pub fn with_code(
        code: Arc<Cell>,
        owner_address: Option<Address>,
        content: NFTCollectionContent,
        royalty_params: RoyaltyParams,
        nft_item_code: Arc<Cell>,
    ) -> Self {
        Self::from_parts(
            code,
            NFTCollectionData {
                owner_address,
                next_item_index: 0,
                content,
                nft_item_code,
                royalty_params,
            },
        )
    }

    /// Creates a collection from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: NFTCollectionData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates a collection from decoded data and embedded default code.
    pub fn from_data(data: NFTCollectionData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::NFTCollectionEditable, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Collection owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Next item index.
    pub fn next_item_index(&self) -> u64 {
        self.spec.data.next_item_index
    }

    /// Collection metadata.
    pub fn content(&self) -> &NFTCollectionContent {
        &self.spec.data.content
    }

    /// Item code published by the collection.
    pub fn nft_item_code(&self) -> &Arc<Cell> {
        &self.spec.data.nft_item_code
    }

    /// Royalty parameters.
    pub fn royalty_params(&self) -> &RoyaltyParams {
        &self.spec.data.royalty_params
    }

    /// Computes an item address using this collection's code and address.
    pub fn item_address(&self, index: u64) -> Result<Address, ContractBuildError> {
        let collection = self.address()?;
        calculate_item_address(
            index,
            &collection,
            &self.spec.data.nft_item_code,
            self.spec.workchain,
        )
    }

    /// Builds a single-item mint body with an editable item mint reference.
    pub fn mint_body(
        &self,
        owner_address: Option<Address>,
        editor_address: Option<Address>,
        content: crate::metadata::Tep64Content,
        item_index: u64,
        forward_ton_amount: u128,
    ) -> Result<super::messages::NftCollectionMintItem, ContractBuildError> {
        let item_ref = super::data::NFTItemEditableMintRef {
            owner_address,
            editor_address,
            content,
        }
        .to_cell()
        .map_err(ContractBuildError::DataSerialization)?;
        Ok(super::messages::NftCollectionMintItem::new(
            item_index,
            forward_ton_amount,
            item_ref,
        ))
    }
}

impl ContractDefinition for NFTCollectionEditable {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::NFTCollectionEditable.default_code()
    }
    type Data = NFTCollectionData;

    fn code(&self) -> &Arc<Cell> {
        &self.spec.code
    }

    fn data(&self) -> Self::Data {
        self.spec.data.clone()
    }

    fn workchain(&self) -> i8 {
        self.spec.workchain
    }
}

/// Computes an NFT item address offline.
///
/// The address depends only on the item index, the collection address, and the
/// item code, matching the reference collection contracts.
pub fn calculate_item_address(
    index: u64,
    collection: &Address,
    nft_item_code: &Arc<Cell>,
    workchain: i8,
) -> Result<Address, ContractBuildError> {
    let mut builder = Builder::new();
    builder
        .store_u64(index)
        .map_err(crate::tlb::TlbError::Tvm)
        .map_err(ContractBuildError::DataSerialization)?;
    store_address_field(&mut builder, Some(collection))
        .map_err(ContractBuildError::DataSerialization)?;
    let data = builder
        .build()
        .map_err(crate::tlb::TlbError::Tvm)
        .map_err(ContractBuildError::DataSerialization)?;
    let state_init = StateInit {
        code: Some(nft_item_code.clone()),
        data: Some(data),
        ..StateInit::empty()
    };
    crate::contracts::address_from_state_init(workchain, &state_init)
        .map_err(ContractBuildError::StateInitSerialization)
}
