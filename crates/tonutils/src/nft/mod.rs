//! NFT contracts (TEP-62 / TEP-85).
//!
//! Editions with different storage or get-methods have separate wrappers:
//!
//! - [`NFTCollectionStandard`] / [`NFTItemStandard`]
//! - [`NFTCollectionEditable`] / [`NFTItemEditable`]
//! - [`NFTItemSoulbound`]
//!
//! Collections expose offline item-address calculation and typed get-methods.
//! Items expose edition-specific get-methods such as editor and revocation
//! data. Message bodies live in [`messages`].
//!
//! ```toml
//! tonutils = { version = "2.1", features = ["nft", "provider"] }
//! ```

pub mod collection;
pub mod data;
pub mod item;
pub mod messages;
mod methods;

pub use collection::{NFTCollectionEditable, NFTCollectionStandard, calculate_item_address};
pub use data::{
    NFTCollectionContent, NFTCollectionData, NFTItemEditableData, NFTItemEditableMintRef,
    NFTItemSoulboundData, NFTItemSoulboundMintRef, NFTItemStandardData, NFTItemStandardMintRef,
    RoyaltyParams,
};
pub use item::{NFTItemEditable, NFTItemSoulbound, NFTItemStandard};
pub use messages::{
    NftCollectionBatchMint, NftCollectionChangeContent, NftCollectionChangeOwner,
    NftCollectionMintItem, NftDestroy, NftEditContent, NftRevoke, NftTransfer,
    NftTransferEditorship,
};
pub use methods::{
    NFTCollectionInfo, NFTItemInfo, get_authority_address, get_collection_data, get_editor,
    get_nft_address_by_index, get_nft_content, get_nft_data, get_revoked_time, get_royalty_params,
};

pub use crate::contracts::ForwardPayload;

/// `transfer#5fcc3d14` operation code.
pub const NFT_TRANSFER_OP: u32 = 0x5fcc_3d14;
/// `ownership_assigned#05138d91` operation code.
pub const NFT_OWNERSHIP_ASSIGNED_OP: u32 = 0x0513_8d91;
/// `get_static_data#2fcb26a2` operation code.
pub const NFT_GET_STATIC_DATA_OP: u32 = 0x2fcb_26a2;
/// `report_static_data#8b771735` operation code.
pub const NFT_REPORT_STATIC_DATA_OP: u32 = 0x8b77_1735;
/// `edit_content#1a0b9d51` operation code.
pub const NFT_EDIT_CONTENT_OP: u32 = 0x1a0b_9d51;
/// `transfer_editorship#1c04412a` operation code.
pub const NFT_TRANSFER_EDITORSHIP_OP: u32 = 0x1c04_412a;
/// `editorship_assigned#511a4463` operation code.
pub const NFT_EDITORSHIP_ASSIGNED_OP: u32 = 0x511a_4463;
/// `get_royalty_params#693d3950` operation code.
pub const NFT_GET_ROYALTY_PARAMS_OP: u32 = 0x693d_3950;
/// `report_royalty_params#a8cb00ad` operation code.
pub const NFT_REPORT_ROYALTY_PARAMS_OP: u32 = 0xa8cb_00ad;
/// `destroy#1f04537a` soulbound operation code.
pub const SBT_DESTROY_OP: u32 = 0x1f04_537a;
/// `revoke#6f89f5e3` soulbound operation code.
pub const SBT_REVOKE_OP: u32 = 0x6f89_f5e3;

#[cfg(test)]
mod tests {
    use crate::contracts::{ContractData, ContractDefinition};
    use crate::metadata::Tep64Content;
    use crate::tvm::{Address, Slice};

    use super::*;

    fn address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn content(uri: &str) -> Tep64Content {
        Tep64Content::off_chain(uri).expect("content encodes")
    }

    fn collection_content() -> NFTCollectionContent {
        NFTCollectionContent::new(
            content("https://example.com/collection.json"),
            content("https://example.com/items/"),
        )
    }

    #[test]
    fn collection_data_round_trips() {
        let data = NFTCollectionData {
            owner_address: Some(address(1)),
            next_item_index: 5,
            content: collection_content(),
            nft_item_code: NFTItemStandard::default_code().expect("code decodes"),
            royalty_params: RoyaltyParams::new(50, 1000, Some(address(2))),
        };
        let cell = <_ as ContractData>::to_cell(&data).expect("data serializes");
        let decoded = <NFTCollectionData as ContractData>::from_cell(&cell).expect("data decodes");
        assert_eq!(decoded, data);
        assert!((decoded.royalty_params.fraction() - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn editable_item_data_round_trips() {
        let data = NFTItemEditableData {
            index: 9,
            collection_address: Some(address(3)),
            owner_address: Some(address(4)),
            content: content("https://example.com/item.json"),
            editor_address: Some(address(5)),
        };
        let cell = <_ as ContractData>::to_cell(&data).expect("data serializes");
        let decoded = <NFTItemEditableData as ContractData>::from_cell(&cell).expect("decodes");
        assert_eq!(decoded, data);
    }

    #[test]
    fn soulbound_item_data_round_trips() {
        let data = NFTItemSoulboundData {
            index: 1,
            collection_address: Some(address(6)),
            owner_address: Some(address(7)),
            content: content("https://example.com/sbt.json"),
            authority_address: Some(address(8)),
            revoked_at: 1_700_000_000,
        };
        let cell = <_ as ContractData>::to_cell(&data).expect("data serializes");
        let decoded = <NFTItemSoulboundData as ContractData>::from_cell(&cell).expect("decodes");
        assert_eq!(decoded, data);
    }

    #[test]
    fn collection_item_address_matches_offline_helper() {
        let collection = NFTCollectionStandard::new(
            Some(address(9)),
            collection_content(),
            RoyaltyParams::new(5, 100, None),
        )
        .expect("collection builds");
        let from_method = collection.item_address(3).expect("address derives");
        let direct = calculate_item_address(
            3,
            &collection.address().expect("collection address"),
            collection.nft_item_code(),
            collection.workchain(),
        )
        .expect("address derives");
        assert_eq!(from_method, direct);
    }

    #[test]
    fn transfer_body_uses_tep62_opcode() {
        let body = NftTransfer::new(address(10));
        let cell = <_ as crate::tlb::TlbSerialize>::to_cell(&body).expect("body serializes");
        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_u32().expect("opcode"), NFT_TRANSFER_OP);
    }

    #[test]
    fn mint_reference_is_stored_as_a_reference() {
        let collection = NFTCollectionStandard::new(
            Some(address(11)),
            collection_content(),
            RoyaltyParams::new(1, 10, None),
        )
        .expect("collection builds");
        let mint = collection
            .mint_body(
                Some(address(12)),
                content("https://example.com/new.json"),
                0,
                1_000_000,
            )
            .expect("mint body builds");
        let cell = <_ as crate::tlb::TlbSerialize>::to_cell(&mint).expect("body serializes");
        assert_eq!(Slice::new(cell).load_u32().expect("opcode"), 1);
    }
}
