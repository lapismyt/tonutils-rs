use super::*;

use super::*;

use super::*;
use crate::tlb::{TlbDeserialize, TlbSerialize};
use crate::tvm::deserialize_boc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;

#[cfg(feature = "liteclient")]
use {
    crate::contracts::ContractProvider,
    crate::tl::{
        BlockIdExt,
        common::{AccountId, Int256},
        response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
    },
    crate::tvm::{TvmStack, TvmStackEntry},
    async_trait::async_trait,
};

#[cfg(feature = "liteclient")]
#[derive(Debug, thiserror::Error)]
#[error("mock provider error")]
pub(super) struct MockProviderError;

#[cfg(feature = "liteclient")]
pub(super) struct WalletGetMockProvider {
    latest: BlockIdExt,
    account: Address,
    result: Result<RunMethodResult, MockProviderError>,
    method_calls: Vec<u64>,
    account_calls: Vec<Address>,
}

#[cfg(feature = "liteclient")]
pub(super) struct WalletSendMockProvider {
    result: Result<u32, MockProviderError>,
    bodies: Vec<Vec<u8>>,
}

#[cfg(feature = "liteclient")]
#[async_trait]
impl ContractProvider for WalletGetMockProvider {
    type Error = MockProviderError;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        Ok(MasterchainInfo {
            last: self.latest.clone(),
            state_root_hash: Int256([1; 32]),
            init: crate::tl::common::ZeroStateIdExt {
                workchain: -1,
                root_hash: Int256([2; 32]),
                file_hash: Int256([3; 32]),
            },
        })
    }

    async fn get_account_state(
        &mut self,
        _block: BlockIdExt,
        _account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        unimplemented!("wallet get-method helpers do not read account state")
    }

    async fn get_account_state_typed(
        &mut self,
        _block: BlockIdExt,
        _account: Address,
    ) -> Result<crate::liteclient::boc::DecodedAccountState, Self::Error> {
        unimplemented!("wallet get-method helpers do not read account state")
    }

    async fn get_account_state_simple(
        &mut self,
        _block: BlockIdExt,
        _account: Address,
    ) -> Result<crate::liteclient::boc::SimpleAccount, Self::Error> {
        unimplemented!("wallet get-method helpers do not read account state")
    }

    async fn run_get_method(
        &mut self,
        _mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        assert_eq!(block, self.latest);
        assert_eq!(account, self.account);
        assert!(stack.entries().is_empty());
        self.method_calls.push(method_id);
        self.account_calls.push(account);
        match &self.result {
            Ok(result) => Ok(result.clone()),
            Err(_) => Err(MockProviderError),
        }
    }

    async fn send_external_message_boc(&mut self, _body: Vec<u8>) -> Result<u32, Self::Error> {
        unimplemented!("wallet get-method helpers do not send messages")
    }

    async fn get_transactions(
        &mut self,
        _count: u32,
        _account: AccountId,
        _lt: u64,
        _hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        unimplemented!("wallet get-method helpers do not read transactions")
    }
}

#[cfg(feature = "liteclient")]
#[async_trait]
impl ContractProvider for WalletSendMockProvider {
    type Error = MockProviderError;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        unimplemented!("wallet send helpers do not read masterchain info")
    }

    async fn get_account_state(
        &mut self,
        _block: BlockIdExt,
        _account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        unimplemented!("wallet send helpers do not read account state")
    }

    async fn get_account_state_typed(
        &mut self,
        _block: BlockIdExt,
        _account: Address,
    ) -> Result<crate::liteclient::boc::DecodedAccountState, Self::Error> {
        unimplemented!("wallet send helpers do not read account state")
    }

    async fn get_account_state_simple(
        &mut self,
        _block: BlockIdExt,
        _account: Address,
    ) -> Result<crate::liteclient::boc::SimpleAccount, Self::Error> {
        unimplemented!("wallet send helpers do not read account state")
    }

    async fn run_get_method(
        &mut self,
        _mode: u32,
        _block: BlockIdExt,
        _account: Address,
        _method_id: u64,
        _stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        unimplemented!("wallet send helpers do not run get-methods")
    }

    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
        self.bodies.push(body);
        match self.result {
            Ok(seqno) => Ok(seqno),
            Err(_) => Err(MockProviderError),
        }
    }

    async fn get_transactions(
        &mut self,
        _count: u32,
        _account: AccountId,
        _lt: u64,
        _hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        unimplemented!("wallet send helpers do not read transactions")
    }
}

pub(super) fn test_code() -> Arc<Cell> {
    let mut builder = Builder::new();
    builder.store_u32(0xfeed_beef).unwrap();
    builder.build().unwrap()
}

pub(super) fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

pub(super) fn extensions_cell(extensions: &WalletV5R1Extensions) -> Arc<Cell> {
    extensions.to_cell().unwrap()
}

pub(super) fn fixture_mnemonic() -> &'static str {
    "open price dish charge law skirt alien churn fire swap number brass outdoor diamond lesson april remain puzzle title elbow valley grant champion staff"
}

#[derive(Debug, Deserialize)]
pub(super) struct WalletFixtureSet {
    pub(super) schema_revision: String,
    pub(super) fixtures: Vec<WalletFixture>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WalletFixture {
    pub(super) name: String,
    pub(super) source: String,
    pub(super) capture_date: String,
    pub(super) upstream_reference: String,
    pub(super) wallet_version: String,
    pub(super) network: String,
    pub(super) public_key: String,
    pub(super) workchain: i8,
    pub(super) wallet_id: String,
    pub(super) code_hash: String,
    pub(super) data_hash: String,
    pub(super) state_init_hash: String,
    pub(super) raw_address: String,
    pub(super) user_friendly_address: String,
}

pub(super) fn wallet_fixture_set() -> WalletFixtureSet {
    let set: WalletFixtureSet = serde_json::from_str(include_str!(
        "../../../fixtures/wallets/state_init_addresses.json"
    ))
    .unwrap();
    assert!(
        set.schema_revision
            .contains("TON wallet state-init/address fixtures")
    );
    assert_eq!(set.fixtures.len(), 3);
    set
}

pub(super) fn hex_32(value: &str) -> [u8; 32] {
    let bytes = hex::decode(value).unwrap();
    bytes.try_into().unwrap()
}

pub(super) fn wallet_id_hex(value: &str) -> u32 {
    u32::from_str_radix(value.strip_prefix("0x").unwrap(), 16).unwrap()
}

pub(super) fn assert_fixture_metadata(fixture: &WalletFixture) {
    assert_eq!(fixture.capture_date, "2026-05-12");
    assert!(fixture.source.contains("deterministic offline fixture"));
    assert!(
        fixture
            .upstream_reference
            .contains("docs.ton.org/standard/wallets/history")
    );
    assert!(
        fixture
            .upstream_reference
            .contains("docs.ton.org/standard/wallets/interact")
    );
    assert!(!fixture.network.is_empty());
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_get_block() -> BlockIdExt {
    BlockIdExt {
        workchain: -1,
        shard: i64::MIN,
        seqno: 42,
        root_hash: Int256([4; 32]),
        file_hash: Int256([5; 32]),
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_get_result(exit_code: i32, result: Option<TvmStack>) -> RunMethodResult {
    RunMethodResult {
        mode: (),
        id: wallet_get_block(),
        shardblk: wallet_get_block(),
        shard_proof: None,
        proof: None,
        state_proof: None,
        init_c7: None,
        lib_extras: None,
        exit_code,
        result: result.map(|stack| stack.to_boc().unwrap()),
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_get_mock(wallet: &WalletV5R1, stack: TvmStack) -> WalletGetMockProvider {
    WalletGetMockProvider {
        latest: wallet_get_block(),
        account: wallet.address().unwrap(),
        result: Ok(wallet_get_result(0, Some(stack))),
        method_calls: Vec::new(),
        account_calls: Vec::new(),
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_get_wallet() -> WalletV5R1 {
    WalletV5R1::new(
        VerifyingKey::from(&signing_key()).to_bytes(),
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        test_code(),
        0,
    )
}

#[cfg(feature = "liteclient")]
pub(super) fn wallet_send_mock(result: Result<u32, MockProviderError>) -> WalletSendMockProvider {
    WalletSendMockProvider {
        result,
        bodies: Vec::new(),
    }
}

#[cfg(feature = "liteclient")]
pub(super) fn assert_external_send_boc(body: &[u8], destination: Address, expect_init: bool) {
    let decoded = Message::from_cell(deserialize_boc(body).unwrap()).unwrap();
    match decoded.info {
        CommonMsgInfo::ExternalIn { dest, .. } => {
            assert_eq!(dest, MsgAddressInt::std(destination));
        }
        _ => panic!("expected external inbound message"),
    }
    assert_eq!(decoded.init.is_some(), expect_init);
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_method_helpers_route_expected_methods() {
    let wallet = wallet_get_wallet();

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(7)]));
    assert_eq!(wallet.seqno(&mut provider).await.unwrap(), 7);
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("seqno")]
    );
    assert_eq!(provider.account_calls, vec![wallet.address().unwrap()]);

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(8)]));
    assert_eq!(wallet.wallet_id_onchain(&mut provider).await.unwrap(), 8);
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("get_wallet_id")]
    );

    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::int(BigInt::from_bytes_be(
            Sign::Plus,
            &[0x11; 32],
        ))]),
    );
    assert_eq!(
        wallet.public_key_onchain(&mut provider).await.unwrap(),
        [0x11; 32]
    );
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("get_public_key")]
    );

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(1)]));
    assert!(
        wallet
            .is_signature_allowed_onchain(&mut provider)
            .await
            .unwrap()
    );
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("is_signature_allowed")]
    );

    let raw_extensions = test_code();
    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::Cell(raw_extensions.clone())]),
    );
    assert_eq!(
        wallet
            .extensions_raw_onchain(&mut provider)
            .await
            .unwrap()
            .hash(),
        raw_extensions.hash()
    );
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("get_extensions")]
    );
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_method_uint32_decoding_rejects_invalid_values() {
    let wallet = wallet_get_wallet();

    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::int(BigInt::from(u32::MAX))]),
    );
    assert_eq!(wallet.seqno(&mut provider).await.unwrap(), u32::MAX);

    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::int(BigInt::from(u32::MAX) + 1)]),
    );
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::IntegerRange {
            method: "seqno",
            expected: "uint32",
            ..
        }
    ));

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(-1)]));
    assert!(matches!(
        wallet.wallet_id_onchain(&mut provider).await.unwrap_err(),
        WalletGetMethodError::IntegerRange {
            method: "get_wallet_id",
            expected: "uint32",
            ..
        }
    ));
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_public_key_decodes_uint256_integer() {
    let wallet = wallet_get_wallet();
    let mut expected = [0u8; 32];
    expected[31] = 0x2a;

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(0x2a)]));
    assert_eq!(
        wallet.public_key_onchain(&mut provider).await.unwrap(),
        expected
    );

    let too_wide = BigInt::from_bytes_be(Sign::Plus, &[1u8; 33]);
    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(too_wide)]));
    assert!(matches!(
        wallet.public_key_onchain(&mut provider).await.unwrap_err(),
        WalletGetMethodError::PublicKeyWidth {
            method: "get_public_key",
            ..
        }
    ));
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_signature_allowed_accepts_only_zero_or_one() {
    let wallet = wallet_get_wallet();

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(0)]));
    assert!(
        !wallet
            .is_signature_allowed_onchain(&mut provider)
            .await
            .unwrap()
    );

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(1)]));
    assert!(
        wallet
            .is_signature_allowed_onchain(&mut provider)
            .await
            .unwrap()
    );

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::int(2)]));
    assert!(matches!(
        wallet
            .is_signature_allowed_onchain(&mut provider)
            .await
            .unwrap_err(),
        WalletGetMethodError::IntegerRange {
            method: "is_signature_allowed",
            expected: "0 or 1",
            ..
        }
    ));
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_extensions_preserves_raw_slice_entry() {
    let wallet = wallet_get_wallet();
    let raw_extensions = test_code();
    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::Slice(raw_extensions.clone())]),
    );

    assert_eq!(
        wallet
            .extensions_raw_onchain(&mut provider)
            .await
            .unwrap()
            .hash(),
        raw_extensions.hash()
    );
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_extensions_decodes_typed_dictionary() {
    let wallet = wallet_get_wallet();
    let mut extensions = WalletV5R1Extensions::empty();
    extensions.insert_hash([0x11; 32]);
    extensions.insert_hash([0x22; 32]);
    let raw = extensions_cell(&extensions);
    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::Cell(raw)]));

    let decoded = wallet.extensions_onchain(&mut provider).await.unwrap();
    assert_eq!(decoded.len(), 2);
    assert!(decoded.contains_hash([0x11; 32]));
    assert!(decoded.contains_hash([0x22; 32]));
    assert_eq!(
        provider.method_calls,
        vec![crate::utils::method_name_to_id("get_extensions")]
    );
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_extensions_decodes_empty_dictionary() {
    let wallet = wallet_get_wallet();
    let raw = extensions_cell(&WalletV5R1Extensions::empty());
    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::Cell(raw)]));

    let decoded = wallet.extensions_onchain(&mut provider).await.unwrap();
    assert!(decoded.is_empty());
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_extensions_rejects_malformed_dictionary() {
    let wallet = wallet_get_wallet();
    let mut builder = Builder::new();
    builder.store_bit(true).unwrap();
    let malformed = builder.build().unwrap();
    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::Cell(malformed.clone())]),
    );

    assert!(matches!(
        wallet.extensions_onchain(&mut provider).await.unwrap_err(),
        WalletGetMethodError::InvalidCell {
            method: "get_extensions",
            ..
        }
    ));

    let mut provider = wallet_get_mock(
        &wallet,
        TvmStack::new(vec![TvmStackEntry::Cell(malformed.clone())]),
    );
    assert_eq!(
        wallet
            .extensions_raw_onchain(&mut provider)
            .await
            .unwrap()
            .hash(),
        malformed.hash()
    );
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_method_helpers_report_stack_failures() {
    let wallet = wallet_get_wallet();

    let mut provider = WalletGetMockProvider {
        latest: wallet_get_block(),
        account: wallet.address().unwrap(),
        result: Ok(wallet_get_result(5, Some(TvmStack::empty()))),
        method_calls: Vec::new(),
        account_calls: Vec::new(),
    };
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::NonZeroExitCode {
            method: "seqno",
            exit_code: 5
        }
    ));

    let mut provider = WalletGetMockProvider {
        latest: wallet_get_block(),
        account: wallet.address().unwrap(),
        result: Ok(wallet_get_result(0, None)),
        method_calls: Vec::new(),
        account_calls: Vec::new(),
    };
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::MissingStack { method: "seqno" }
    ));

    let mut provider = WalletGetMockProvider {
        latest: wallet_get_block(),
        account: wallet.address().unwrap(),
        result: Ok(RunMethodResult {
            result: Some(vec![0xff]),
            ..wallet_get_result(0, None)
        }),
        method_calls: Vec::new(),
        account_calls: Vec::new(),
    };
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::UndecodableStack {
            method: "seqno",
            ..
        }
    ));

    let mut provider = wallet_get_mock(&wallet, TvmStack::empty());
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::MissingStackEntry {
            method: "seqno",
            index: 0
        }
    ));

    let mut provider = wallet_get_mock(&wallet, TvmStack::new(vec![TvmStackEntry::Null]));
    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::WrongStackType {
            method: "seqno",
            expected: "integer",
            ..
        }
    ));
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_get_method_helpers_propagate_provider_errors() {
    let wallet = wallet_get_wallet();
    let mut provider = WalletGetMockProvider {
        latest: wallet_get_block(),
        account: wallet.address().unwrap(),
        result: Err(MockProviderError),
        method_calls: Vec::new(),
        account_calls: Vec::new(),
    };

    assert!(matches!(
        wallet.seqno(&mut provider).await.unwrap_err(),
        WalletGetMethodError::Provider(_)
    ));
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v5r1_send_external_message_routes_one_boc_and_returns_provider_result() {
    let key = signing_key();
    let wallet = WalletV5R1::new(
        VerifyingKey::from(&key).to_bytes(),
        WALLET_V5R1_MAINNET_DEFAULT_ID,
        test_code(),
        0,
    );
    let mut provider = wallet_send_mock(Ok(43));

    let result = wallet
        .send_external_message(&mut provider, 42, 1_700_000_001, Vec::new(), &key, true)
        .await
        .unwrap();

    assert_eq!(result, 43);
    assert_eq!(provider.bodies.len(), 1);
    assert_external_send_boc(&provider.bodies[0], wallet.address().unwrap(), true);
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn v4r2_send_external_message_routes_one_boc_and_returns_provider_result() {
    let key = signing_key();
    let wallet = WalletV4R2::new(
        VerifyingKey::from(&key).to_bytes(),
        WALLET_V4R2_DEFAULT_ID,
        test_code(),
        0,
    );
    let mut provider = wallet_send_mock(Ok(8));

    let result = wallet
        .send_external_message(&mut provider, 7, 1_700_000_001, Vec::new(), &key, true)
        .await
        .unwrap();

    assert_eq!(result, 8);
    assert_eq!(provider.bodies.len(), 1);
    assert_external_send_boc(&provider.bodies[0], wallet.address().unwrap(), true);
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn wallet_send_external_message_preserves_state_init_choice() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key).to_bytes();
    let v5 = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let v4 = WalletV4R2::new(public_key, WALLET_V4R2_DEFAULT_ID, test_code(), 0);

    let mut provider = wallet_send_mock(Ok(1));
    v5.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, true)
        .await
        .unwrap();
    assert_external_send_boc(&provider.bodies[0], v5.address().unwrap(), true);

    let mut provider = wallet_send_mock(Ok(1));
    v5.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, false)
        .await
        .unwrap();
    assert_external_send_boc(&provider.bodies[0], v5.address().unwrap(), false);

    let mut provider = wallet_send_mock(Ok(1));
    v4.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, true)
        .await
        .unwrap();
    assert_external_send_boc(&provider.bodies[0], v4.address().unwrap(), true);

    let mut provider = wallet_send_mock(Ok(1));
    v4.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, false)
        .await
        .unwrap();
    assert_external_send_boc(&provider.bodies[0], v4.address().unwrap(), false);
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn wallet_send_external_message_propagates_provider_errors() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key).to_bytes();

    let v5 = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let mut provider = wallet_send_mock(Err(MockProviderError));
    assert!(matches!(
        v5.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, true)
            .await
            .unwrap_err(),
        WalletSendError::Provider(_)
    ));
    assert_eq!(provider.bodies.len(), 1);
    assert_external_send_boc(&provider.bodies[0], v5.address().unwrap(), true);

    let v4 = WalletV4R2::new(public_key, WALLET_V4R2_DEFAULT_ID, test_code(), 0);
    let mut provider = wallet_send_mock(Err(MockProviderError));
    assert!(matches!(
        v4.send_external_message(&mut provider, 0, 1_700_000_001, Vec::new(), &key, true)
            .await
            .unwrap_err(),
        WalletSendError::Provider(_)
    ));
    assert_eq!(provider.bodies.len(), 1);
    assert_external_send_boc(&provider.bodies[0], v4.address().unwrap(), true);
}

#[cfg(feature = "liteclient")]
#[tokio::test]
async fn wallet_send_external_message_build_errors_do_not_call_provider() {
    let key = signing_key();
    let public_key = VerifyingKey::from(&key).to_bytes();

    let v5 = WalletV5R1::new(public_key, WALLET_V5R1_MAINNET_DEFAULT_ID, test_code(), 0);
    let mut provider = wallet_send_mock(Ok(1));
    let messages = vec![WalletMessage::internal(Address::new(0, [1; 32]), 1); 256];
    assert!(matches!(
        v5.send_external_message(&mut provider, 0, 1_700_000_001, messages, &key, true)
            .await
            .unwrap_err(),
        WalletSendError::Build(WalletError::TooManyActions {
            count: 256,
            max: 255
        })
    ));
    assert!(provider.bodies.is_empty());

    let v4 = WalletV4R2::new(public_key, WALLET_V4R2_DEFAULT_ID, test_code(), 0);
    let mut provider = wallet_send_mock(Ok(1));
    let messages = vec![WalletMessage::internal(Address::new(0, [1; 32]), 1); 5];
    assert!(matches!(
        v4.send_external_message(&mut provider, 0, 1_700_000_001, messages, &key, true)
            .await
            .unwrap_err(),
        WalletSendError::Build(WalletError::TooManyActions { count: 5, max: 4 })
    ));
    assert!(provider.bodies.is_empty());
}
