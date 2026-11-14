//! Typed NFT get-method wrappers.

use std::sync::Arc;

use num_bigint::BigInt;

use crate::{
    contracts::{
        ContractDefinition, ContractError, ensure_consumed, next_entry as next,
        run_get_method as run,
    },
    metadata::Tep64Content,
    provider::Provider,
    tvm::{Address, Cell},
};

use super::{
    collection::{NFTCollectionEditable, NFTCollectionStandard},
    data::{NFTCollectionContent, RoyaltyParams},
    item::{NFTItemEditable, NFTItemSoulbound, NFTItemStandard},
};

/// Decoded `get_collection_data` result.
#[derive(Debug, Clone)]
pub struct NFTCollectionInfo {
    /// Index that will be assigned to the next item.
    pub next_item_index: u64,
    /// Collection metadata.
    pub content: NFTCollectionContent,
    /// Collection owner.
    pub owner_address: Option<Address>,
}

/// Decoded `get_nft_data` result.
#[derive(Debug, Clone)]
pub struct NFTItemInfo {
    /// Whether the item is initialized.
    pub initialized: bool,
    /// Item index.
    pub index: u64,
    /// Parent collection.
    pub collection_address: Option<Address>,
    /// Current owner.
    pub owner_address: Option<Address>,
    /// Item metadata.
    pub content: Tep64Content,
}

/// Runs `get_collection_data`.
pub async fn get_collection_data<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<NFTCollectionInfo, ContractError> {
    let entries = run(provider, address, "get_collection_data", ()).await?;
    let mut entries = entries.into_iter();
    let next_item_index = next(&mut entries)?;
    let content_cell: Arc<Cell> = next(&mut entries)?;
    let owner_address = next(&mut entries)?;
    ensure_consumed(&entries)?;
    let content = NFTCollectionContent::from_cell(&content_cell)
        .map_err(|error| ContractError::Data(Box::new(error)))?;
    Ok(NFTCollectionInfo {
        next_item_index,
        content,
        owner_address,
    })
}

/// Runs `get_nft_address_by_index`.
pub async fn get_nft_address_by_index<P: Provider + ?Sized>(
    provider: &mut P,
    collection: Address,
    index: u64,
) -> Result<Address, ContractError> {
    let entries = run(provider, collection, "get_nft_address_by_index", index).await?;
    let mut entries = entries.into_iter();
    let address = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(address)
}

/// Runs `get_nft_content`.
pub async fn get_nft_content<P: Provider + ?Sized>(
    provider: &mut P,
    collection: Address,
    index: u64,
    individual_content: Arc<Cell>,
) -> Result<Arc<Cell>, ContractError> {
    let entries = run(
        provider,
        collection,
        "get_nft_content",
        (index, individual_content),
    )
    .await?;
    let mut entries = entries.into_iter();
    let content = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(content)
}

/// Runs `royalty_params`.
pub async fn get_royalty_params<P: Provider + ?Sized>(
    provider: &mut P,
    collection: Address,
) -> Result<RoyaltyParams, ContractError> {
    let entries = run(provider, collection, "royalty_params", ()).await?;
    let mut entries = entries.into_iter();
    let numerator = next(&mut entries)?;
    let denominator = next(&mut entries)?;
    let address = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(RoyaltyParams {
        numerator,
        denominator,
        address,
    })
}

/// Runs `get_nft_data`.
pub async fn get_nft_data<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<NFTItemInfo, ContractError> {
    let entries = run(provider, address, "get_nft_data", ()).await?;
    let mut entries = entries.into_iter();
    let initialized: BigInt = next(&mut entries)?;
    let index = next(&mut entries)?;
    let collection_address = next(&mut entries)?;
    let owner_address = next(&mut entries)?;
    let content_cell: Arc<Cell> = next(&mut entries)?;
    ensure_consumed(&entries)?;
    let content = crate::metadata::parse_tep64_offchain(content_cell)
        .map_err(|error| ContractError::Data(Box::new(error)))?;
    Ok(NFTItemInfo {
        initialized: initialized != BigInt::from(0),
        index,
        collection_address,
        owner_address,
        content,
    })
}

/// Runs `get_editor`.
pub async fn get_editor<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<Option<Address>, ContractError> {
    let entries = run(provider, address, "get_editor", ()).await?;
    let mut entries = entries.into_iter();
    let editor = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(editor)
}

/// Runs `get_authority_address`.
pub async fn get_authority_address<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<Option<Address>, ContractError> {
    let entries = run(provider, address, "get_authority_address", ()).await?;
    let mut entries = entries.into_iter();
    let authority = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(authority)
}

/// Runs `get_revoked_time`.
pub async fn get_revoked_time<P: Provider + ?Sized>(
    provider: &mut P,
    address: Address,
) -> Result<u64, ContractError> {
    let entries = run(provider, address, "get_revoked_time", ()).await?;
    let mut entries = entries.into_iter();
    let revoked_at = next(&mut entries)?;
    ensure_consumed(&entries)?;
    Ok(revoked_at)
}

macro_rules! collection_getters {
    ($ty:ty) => {
        impl $ty {
            /// Runs `get_collection_data`.
            pub async fn get_collection_data<P: Provider + ?Sized>(
                &self,
                provider: &mut P,
            ) -> Result<NFTCollectionInfo, ContractError> {
                get_collection_data(provider, self.address()?).await
            }

            /// Runs `get_nft_address_by_index`.
            pub async fn get_nft_address_by_index<P: Provider + ?Sized>(
                &self,
                provider: &mut P,
                index: u64,
            ) -> Result<Address, ContractError> {
                get_nft_address_by_index(provider, self.address()?, index).await
            }

            /// Runs `get_nft_content`.
            pub async fn get_nft_content<P: Provider + ?Sized>(
                &self,
                provider: &mut P,
                index: u64,
                individual_content: Arc<Cell>,
            ) -> Result<Arc<Cell>, ContractError> {
                get_nft_content(provider, self.address()?, index, individual_content).await
            }

            /// Runs `royalty_params`.
            pub async fn get_royalty_params<P: Provider + ?Sized>(
                &self,
                provider: &mut P,
            ) -> Result<RoyaltyParams, ContractError> {
                get_royalty_params(provider, self.address()?).await
            }
        }
    };
}

collection_getters!(NFTCollectionStandard);
collection_getters!(NFTCollectionEditable);

macro_rules! item_getters {
    ($ty:ty) => {
        impl $ty {
            /// Runs `get_nft_data`.
            pub async fn get_nft_data<P: Provider + ?Sized>(
                &self,
                provider: &mut P,
            ) -> Result<NFTItemInfo, ContractError> {
                get_nft_data(provider, self.address()?).await
            }
        }
    };
}

item_getters!(NFTItemStandard);
item_getters!(NFTItemEditable);
item_getters!(NFTItemSoulbound);

impl NFTItemEditable {
    /// Runs `get_editor`.
    pub async fn get_editor<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<Option<Address>, ContractError> {
        get_editor(provider, self.address()?).await
    }
}

impl NFTItemSoulbound {
    /// Runs `get_authority_address`.
    pub async fn get_authority_address<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<Option<Address>, ContractError> {
        get_authority_address(provider, self.address()?).await
    }

    /// Runs `get_revoked_time`.
    pub async fn get_revoked_time<P: Provider + ?Sized>(
        &self,
        provider: &mut P,
    ) -> Result<u64, ContractError> {
        get_revoked_time(provider, self.address()?).await
    }
}
