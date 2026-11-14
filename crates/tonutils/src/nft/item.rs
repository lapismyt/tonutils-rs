//! NFT item wrappers.

use std::sync::Arc;

use crate::{
    contracts::{ContractCodeError, ContractDefinition, ContractSpec, ContractVersion},
    metadata::Tep64Content,
    tvm::{Address, Cell},
};

use super::data::{NFTItemEditableData, NFTItemSoulboundData, NFTItemStandardData};

/// Standard NFT item (TEP-62).
#[derive(Debug, Clone)]
pub struct NFTItemStandard {
    spec: ContractSpec<NFTItemStandardData>,
}

impl NFTItemStandard {
    /// Creates an item with the embedded default code.
    pub fn new(
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::NFTItemStandard,
                NFTItemStandardData {
                    index,
                    collection_address: Some(collection_address),
                    owner_address,
                    content,
                },
            )?,
        })
    }

    /// Creates an item with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
    ) -> Self {
        Self::from_parts(
            code,
            NFTItemStandardData {
                index,
                collection_address: Some(collection_address),
                owner_address,
                content,
            },
        )
    }

    /// Creates an item from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: NFTItemStandardData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates an item from decoded data and embedded default code.
    pub fn from_data(data: NFTItemStandardData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::NFTItemStandard, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Item index.
    pub fn index(&self) -> u64 {
        self.spec.data.index
    }

    /// Parent collection.
    pub fn collection_address(&self) -> Option<&Address> {
        self.spec.data.collection_address.as_ref()
    }

    /// Current owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Item metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }
}

impl ContractDefinition for NFTItemStandard {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::NFTItemStandard.default_code()
    }
    type Data = NFTItemStandardData;

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

/// Editable NFT item (TEP-62).
#[derive(Debug, Clone)]
pub struct NFTItemEditable {
    spec: ContractSpec<NFTItemEditableData>,
}

impl NFTItemEditable {
    /// Creates an item with the embedded default code.
    pub fn new(
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
        editor_address: Option<Address>,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::NFTItemEditable,
                NFTItemEditableData {
                    index,
                    collection_address: Some(collection_address),
                    owner_address,
                    content,
                    editor_address,
                },
            )?,
        })
    }

    /// Creates an item with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
        editor_address: Option<Address>,
    ) -> Self {
        Self::from_parts(
            code,
            NFTItemEditableData {
                index,
                collection_address: Some(collection_address),
                owner_address,
                content,
                editor_address,
            },
        )
    }

    /// Creates an item from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: NFTItemEditableData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates an item from decoded data and embedded default code.
    pub fn from_data(data: NFTItemEditableData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::NFTItemEditable, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Item index.
    pub fn index(&self) -> u64 {
        self.spec.data.index
    }

    /// Parent collection.
    pub fn collection_address(&self) -> Option<&Address> {
        self.spec.data.collection_address.as_ref()
    }

    /// Current owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Item metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }

    /// Address authorized to edit content.
    pub fn editor_address(&self) -> Option<&Address> {
        self.spec.data.editor_address.as_ref()
    }
}

impl ContractDefinition for NFTItemEditable {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::NFTItemEditable.default_code()
    }
    type Data = NFTItemEditableData;

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

/// Soulbound NFT item (TEP-85).
#[derive(Debug, Clone)]
pub struct NFTItemSoulbound {
    spec: ContractSpec<NFTItemSoulboundData>,
}

impl NFTItemSoulbound {
    /// Creates an item with the embedded default code.
    pub fn new(
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
        authority_address: Option<Address>,
        revoked_at: u64,
    ) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(
                ContractVersion::NFTItemSoulbound,
                NFTItemSoulboundData {
                    index,
                    collection_address: Some(collection_address),
                    owner_address,
                    content,
                    authority_address,
                    revoked_at,
                },
            )?,
        })
    }

    /// Creates an item with explicit code.
    pub fn with_code(
        code: Arc<Cell>,
        index: u64,
        collection_address: Address,
        owner_address: Option<Address>,
        content: Tep64Content,
        authority_address: Option<Address>,
        revoked_at: u64,
    ) -> Self {
        Self::from_parts(
            code,
            NFTItemSoulboundData {
                index,
                collection_address: Some(collection_address),
                owner_address,
                content,
                authority_address,
                revoked_at,
            },
        )
    }

    /// Creates an item from explicit code and decoded data.
    pub fn from_parts(code: Arc<Cell>, data: NFTItemSoulboundData) -> Self {
        Self {
            spec: ContractSpec::with_code(code, data),
        }
    }

    /// Creates an item from decoded data and embedded default code.
    pub fn from_data(data: NFTItemSoulboundData) -> Result<Self, ContractCodeError> {
        Ok(Self {
            spec: ContractSpec::new(ContractVersion::NFTItemSoulbound, data)?,
        })
    }

    /// Overrides the workchain used for address derivation.
    #[must_use]
    pub fn with_workchain(mut self, workchain: i8) -> Self {
        self.spec.workchain = workchain;
        self
    }

    /// Item index.
    pub fn index(&self) -> u64 {
        self.spec.data.index
    }

    /// Parent collection.
    pub fn collection_address(&self) -> Option<&Address> {
        self.spec.data.collection_address.as_ref()
    }

    /// Current owner.
    pub fn owner_address(&self) -> Option<&Address> {
        self.spec.data.owner_address.as_ref()
    }

    /// Item metadata.
    pub fn content(&self) -> &Tep64Content {
        &self.spec.data.content
    }

    /// Authority allowed to revoke the token.
    pub fn authority_address(&self) -> Option<&Address> {
        self.spec.data.authority_address.as_ref()
    }

    /// Revocation timestamp, or zero.
    pub fn revoked_at(&self) -> u64 {
        self.spec.data.revoked_at
    }
}

impl ContractDefinition for NFTItemSoulbound {
    fn default_code() -> Result<Arc<Cell>, ContractCodeError> {
        ContractVersion::NFTItemSoulbound.default_code()
    }
    type Data = NFTItemSoulboundData;

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
