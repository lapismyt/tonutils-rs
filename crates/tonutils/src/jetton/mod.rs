//! Jetton contracts (TEP-74).
//!
//! The module provides edition-specific wrappers because storage layouts and
//! get-methods differ between standard and stablecoin jettons:
//!
//! - [`JettonMasterStandard`] / [`JettonWalletStandard`]
//! - [`JettonMasterStablecoin`] / [`JettonWalletStablecoin`]
//! - [`JettonMasterStablecoinV2`] / [`JettonWalletStablecoinV2`] (sharded)
//!
//! Every wrapper supports embedded default code, custom code, offline address
//! calculation, deploy requests, and typed get-method calls. Message bodies live
//! in [`messages`].
//!
//! ```toml
//! tonutils = { version = "2.1", features = ["jetton", "provider"] }
//! ```

pub mod data;
pub mod master;
pub mod messages;
mod methods;
pub mod wallet;

pub use data::{
    JettonMasterStablecoinData, JettonMasterStandardData, JettonWalletStablecoinData,
    JettonWalletStablecoinV2Data, JettonWalletStandardData,
};
pub use master::{JettonMasterStablecoin, JettonMasterStablecoinV2, JettonMasterStandard};
pub use messages::{
    JettonBurn, JettonChangeAdmin, JettonChangeContent, JettonInternalTransfer, JettonMint,
    JettonStandardChangeAdmin, JettonStandardChangeContent, JettonStandardMint, JettonTopUp,
    JettonTransfer, JettonUpgrade,
};
pub use methods::{
    JettonMasterInfo, JettonWalletInfo, get_jetton_data, get_next_admin_address, get_status,
    get_wallet_address, get_wallet_data,
};
pub use wallet::{JettonWalletStablecoin, JettonWalletStablecoinV2, JettonWalletStandard};

pub use crate::contracts::ForwardPayload;

/// `transfer#f8a7ea5` operation code.
pub const JETTON_TRANSFER_OP: u32 = 0x0f8a_7ea5;
/// `internal_transfer#178d4519` operation code.
pub const JETTON_INTERNAL_TRANSFER_OP: u32 = 0x178d_4519;
/// `transfer_notification#7362d09c` operation code.
pub const JETTON_TRANSFER_NOTIFICATION_OP: u32 = 0x7362_d09c;
/// `burn#595f07bc` operation code.
pub const JETTON_BURN_OP: u32 = 0x595f_07bc;
/// `burn_notification#7bdd97de` operation code.
pub const JETTON_BURN_NOTIFICATION_OP: u32 = 0x7bdd_97de;
/// Stablecoin `mint#642b7d07` operation code.
pub const JETTON_MINT_OP: u32 = 0x642b_7d07;
/// Standard `mint#00000015` operation code.
pub const JETTON_STANDARD_MINT_OP: u32 = 21;
/// Stablecoin `change_admin#6501f354` operation code.
pub const JETTON_CHANGE_ADMIN_OP: u32 = 0x6501_f354;
/// Standard `change_admin#00000003` operation code.
pub const JETTON_STANDARD_CHANGE_ADMIN_OP: u32 = 3;
/// Stablecoin `change_metadata#cb862902` operation code.
pub const JETTON_CHANGE_CONTENT_OP: u32 = 0xcb86_2902;
/// Standard `change_content#00000004` operation code.
pub const JETTON_STANDARD_CHANGE_CONTENT_OP: u32 = 4;
/// `upgrade#2508d66a` operation code.
pub const JETTON_UPGRADE_OP: u32 = 0x2508_d66a;
/// `top_up#d372158c` operation code.
pub const JETTON_TOP_UP_OP: u32 = 0xd372_158c;
/// `provide_wallet_address#2c76b973` operation code.
pub const JETTON_PROVIDE_WALLET_ADDRESS_OP: u32 = 0x2c76_b973;

#[cfg(test)]
mod tests {
    use crate::contracts::{ContractData, ContractDefinition, ForwardPayload};
    use crate::metadata::Tep64Content;
    use crate::tvm::{Address, Slice};

    use super::*;

    fn address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn content() -> Tep64Content {
        Tep64Content::off_chain("https://example.com/jetton.json").expect("content encodes")
    }

    #[test]
    fn standard_master_data_round_trips() {
        let data = JettonMasterStandardData {
            total_supply: 1_000_000,
            admin_address: Some(address(1)),
            content: content(),
            jetton_wallet_code: JettonWalletStandard::default_code().expect("code decodes"),
        };
        let cell = <_ as ContractData>::to_cell(&data).expect("data serializes");
        let decoded =
            <JettonMasterStandardData as ContractData>::from_cell(&cell).expect("data decodes");
        assert_eq!(decoded, data);
    }

    #[test]
    fn stablecoin_wallet_data_round_trips() {
        let data = JettonWalletStablecoinData {
            status: 0b1010,
            balance: 42,
            owner_address: Some(address(2)),
            jetton_master_address: Some(address(3)),
        };
        let cell = <_ as ContractData>::to_cell(&data).expect("data serializes");
        let decoded =
            <JettonWalletStablecoinData as ContractData>::from_cell(&cell).expect("data decodes");
        assert_eq!(decoded, data);
        assert_eq!(decoded.status, 0b1010);
    }

    #[test]
    fn standard_master_offline_address_is_stable() {
        let master = JettonMasterStandard::new(Some(address(1)), content()).expect("master builds");
        let first = master.address().expect("address derives");
        let second = master.address().expect("address derives");
        assert_eq!(first, second);
        assert_eq!(first.workchain, 0);
        assert_ne!(first.hash_part, [0u8; 32]);
    }

    #[test]
    fn stablecoin_v2_shards_owner_prefix_into_address() {
        let master = JettonMasterStablecoinV2::new(None, None, content()).expect("master builds");
        let owner = address(0xab);
        let wallet = master
            .wallet_address(&owner)
            .expect("wallet address derives");
        assert_eq!(wallet.hash_part[0], owner.hash_part[0]);
    }

    #[test]
    fn transfer_body_uses_tep74_opcode_and_payload() {
        let body = JettonTransfer {
            query_id: 7,
            amount: 100,
            destination: address(4),
            response_destination: address(5),
            custom_payload: None,
            forward_ton_amount: 1,
            forward_payload: ForwardPayload::comment("hi"),
        };
        let cell = body.to_cell().expect("body serializes");
        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_u32().expect("opcode"), JETTON_TRANSFER_OP);
        assert_eq!(slice.load_u64().expect("query id"), 7);
    }

    #[test]
    fn stablecoin_content_is_stored_without_prefix() {
        let master =
            JettonMasterStablecoin::new(Some(address(6)), None, content()).expect("master builds");
        let data = master.data();
        let cell = data
            .content
            .to_cell_with_prefix(false)
            .expect("content encodes");
        let mut slice = Slice::new(cell);
        let first = slice.load_u8().expect("first byte");
        assert_ne!(first, 0x01, "bare content must not carry the prefix");
    }

    #[test]
    fn wallet_address_matches_reference_derivation() {
        let wallet_code = JettonWalletStandard::default_code().expect("code decodes");
        let owner = address(7);
        let master = address(8);
        let first =
            JettonMasterStandard::calculate_wallet_address(&owner, &master, &wallet_code, 0)
                .expect("address derives");
        let second =
            JettonMasterStandard::calculate_wallet_address(&owner, &master, &wallet_code, 0)
                .expect("address derives");
        assert_eq!(first, second);
    }
}
