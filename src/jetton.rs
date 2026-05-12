//! TEP-74 jetton metadata helpers.
//!
//! This module decodes the `get_jetton_data()` stack returned by jetton master
//! contracts and maps the contained TEP-64 content into jetton-oriented fields.
//! Off-chain JSON fetching is intentionally outside this layer.

use crate::metadata::{MetadataError, Tep64Content, Tep64Field, Tep64KnownKey, Tep64Value};
use crate::tlb::{MsgAddress, MsgAddressExt, MsgAddressInt, TlbDeserialize, ensure_empty};
use crate::tvm::{Address, Cell, Slice, TvmStack, TvmStackEntry};
use num_bigint::{BigInt, BigUint};
use std::sync::Arc;
use thiserror::Error;

const JETTON_DATA_STACK_LEN: usize = 5;

/// Typed result of a TEP-74 jetton master `get_jetton_data()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMasterData {
    /// Total supply in indivisible jetton units.
    pub total_supply: BigUint,
    /// `true` when the contract reports `-1`; `false` when it reports `0`.
    pub mintable: bool,
    /// Standard internal admin address, or `None` for `addr_none`.
    pub admin_address: Option<Address>,
    /// Raw parsed TEP-64 content cell.
    pub jetton_content: Tep64Content,
    /// Jetton wallet code cell returned by the master.
    pub jetton_wallet_code: Arc<Cell>,
}

impl JettonMasterData {
    /// Maps the parsed TEP-64 content into jetton-friendly metadata fields.
    pub fn metadata(&self) -> JettonMetadata {
        JettonMetadata::from_content(self.jetton_content.clone())
    }
}

/// Jetton metadata fields recognized by common TEP-64 keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMetadata {
    pub uri: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub amount_style: Option<String>,
    pub raw_content: Tep64Content,
    pub unknown_fields: Vec<JettonMetadataUnknownField>,
    pub field_diagnostics: Vec<JettonMetadataFieldDiagnostic>,
}

impl JettonMetadata {
    /// Builds typed jetton metadata from already-parsed TEP-64 content.
    pub fn from_content(raw_content: Tep64Content) -> Self {
        let mut metadata = Self {
            uri: None,
            name: None,
            description: None,
            image: None,
            image_data: None,
            symbol: None,
            decimals: None,
            amount_style: None,
            raw_content: raw_content.clone(),
            unknown_fields: Vec::new(),
            field_diagnostics: Vec::new(),
        };

        match &raw_content {
            Tep64Content::OffChain { uri, .. } => {
                metadata.uri = metadata.string_from_bytes(
                    Tep64KnownKey::Uri.key_hash(),
                    Some(Tep64KnownKey::Uri),
                    uri,
                    None,
                );
            }
            Tep64Content::OnChain { fields, .. } => {
                for field in fields {
                    metadata.apply_field(field);
                }
            }
            Tep64Content::Unsupported { .. } => {}
        }

        metadata
    }

    fn apply_field(&mut self, field: &Tep64Field) {
        let Some(known_key) = field.known_key else {
            self.unknown_fields.push(JettonMetadataUnknownField {
                key_hash: field.key_hash,
                raw: field.raw.clone(),
                value: field.value.clone(),
            });
            return;
        };

        match known_key {
            Tep64KnownKey::Uri => {
                self.uri = self.field_string(field);
            }
            Tep64KnownKey::Name => {
                self.name = self.field_string(field);
            }
            Tep64KnownKey::Description => {
                self.description = self.field_string(field);
            }
            Tep64KnownKey::Image => {
                self.image = self.field_string(field);
            }
            Tep64KnownKey::ImageData => {
                self.image_data = self.field_bytes(field);
            }
            Tep64KnownKey::Symbol => {
                self.symbol = self.field_string(field);
            }
            Tep64KnownKey::Decimals => {
                self.decimals = self.field_decimals(field);
            }
            Tep64KnownKey::AmountStyle => {
                self.amount_style = self.field_string(field);
            }
            Tep64KnownKey::RenderType | Tep64KnownKey::ContentUrl | Tep64KnownKey::Video => {
                self.unknown_fields.push(JettonMetadataUnknownField {
                    key_hash: field.key_hash,
                    raw: field.raw.clone(),
                    value: field.value.clone(),
                });
            }
        }
    }

    fn field_string(&mut self, field: &Tep64Field) -> Option<String> {
        let bytes = self.field_bytes(field)?;
        self.string_from_bytes(
            field.key_hash,
            field.known_key,
            &bytes,
            Some(field.raw.clone()),
        )
    }

    fn field_decimals(&mut self, field: &Tep64Field) -> Option<u8> {
        let bytes = self.field_bytes(field)?;
        let raw = Some(field.raw.clone());
        let text = self.string_from_bytes(field.key_hash, field.known_key, &bytes, raw.clone())?;
        match text.parse::<u8>() {
            Ok(value) => Some(value),
            Err(error) => {
                self.field_diagnostics.push(JettonMetadataFieldDiagnostic {
                    key_hash: field.key_hash,
                    known_key: field.known_key,
                    error: format!("invalid decimals value: {error}"),
                    raw: field.raw.clone(),
                });
                None
            }
        }
    }

    fn field_bytes(&mut self, field: &Tep64Field) -> Option<Vec<u8>> {
        match &field.value {
            Tep64Value::Snake(bytes) => Some(bytes.clone()),
            Tep64Value::Chunked { bytes, .. } => Some(bytes.clone()),
            Tep64Value::Unsupported { .. } => {
                self.field_diagnostics.push(JettonMetadataFieldDiagnostic {
                    key_hash: field.key_hash,
                    known_key: field.known_key,
                    error: "unsupported TEP-64 field value encoding".to_string(),
                    raw: field.raw.clone(),
                });
                None
            }
            Tep64Value::Malformed { error } => {
                self.field_diagnostics.push(JettonMetadataFieldDiagnostic {
                    key_hash: field.key_hash,
                    known_key: field.known_key,
                    error: error.clone(),
                    raw: field.raw.clone(),
                });
                None
            }
        }
    }

    fn string_from_bytes(
        &mut self,
        key_hash: [u8; 32],
        known_key: Option<Tep64KnownKey>,
        bytes: &[u8],
        raw: Option<Arc<Cell>>,
    ) -> Option<String> {
        match String::from_utf8(bytes.to_vec()) {
            Ok(value) => Some(value),
            Err(error) => {
                self.field_diagnostics.push(JettonMetadataFieldDiagnostic {
                    key_hash,
                    known_key,
                    error: format!("metadata field is not UTF-8: {error}"),
                    raw: raw.unwrap_or_else(empty_cell),
                });
                None
            }
        }
    }
}

/// Raw-preserved metadata field that is not mapped to a jetton typed field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMetadataUnknownField {
    pub key_hash: [u8; 32],
    pub raw: Arc<Cell>,
    pub value: Tep64Value,
}

/// Field-level metadata diagnostic that does not invalidate the full metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JettonMetadataFieldDiagnostic {
    pub key_hash: [u8; 32],
    pub known_key: Option<Tep64KnownKey>,
    pub error: String,
    pub raw: Arc<Cell>,
}

/// Errors returned while decoding TEP-74 jetton master data.
#[derive(Debug, Error)]
pub enum JettonMetadataError {
    #[error("get_jetton_data stack has {actual} entries, expected {expected}")]
    StackLength { actual: usize, expected: usize },
    #[error("get_jetton_data stack entry {index} has type {actual}, expected {expected}")]
    StackType {
        index: usize,
        expected: &'static str,
        actual: &'static str,
    },
    #[error("get_jetton_data stack integer {field} is invalid: {reason}")]
    InvalidInteger { field: &'static str, reason: String },
    #[error("get_jetton_data admin address is malformed: {0}")]
    MalformedAddress(String),
    #[error("get_jetton_data content cell is malformed: {0}")]
    MalformedContent(#[from] MetadataError),
}

/// Decodes the successful stack returned by TEP-74 `get_jetton_data()`.
pub fn decode_jetton_master_data(
    stack: Vec<TvmStackEntry>,
) -> Result<JettonMasterData, JettonMetadataError> {
    if stack.len() != JETTON_DATA_STACK_LEN {
        return Err(JettonMetadataError::StackLength {
            actual: stack.len(),
            expected: JETTON_DATA_STACK_LEN,
        });
    }

    let total_supply = parse_supply(stack_entry_int(&stack, 0, "total_supply")?)?;
    let mintable = parse_mintable(stack_entry_int(&stack, 1, "mintable")?)?;
    let admin_address = parse_admin_address(stack_entry_cell_like(&stack, 2, "slice/cell")?)?;
    let jetton_content = crate::metadata::parse_tep64_content(stack_entry_cell(&stack, 3)?)?;
    let jetton_wallet_code = stack_entry_cell(&stack, 4)?;

    Ok(JettonMasterData {
        total_supply,
        mintable,
        admin_address,
        jetton_content,
        jetton_wallet_code,
    })
}

/// Decodes a `TvmStack` containing `get_jetton_data()` output.
pub fn decode_jetton_master_data_stack(
    stack: TvmStack,
) -> Result<JettonMasterData, JettonMetadataError> {
    decode_jetton_master_data(stack.entries().to_vec())
}

fn parse_supply(value: &BigInt) -> Result<BigUint, JettonMetadataError> {
    value
        .to_biguint()
        .ok_or_else(|| JettonMetadataError::InvalidInteger {
            field: "total_supply",
            reason: "value must be non-negative".to_string(),
        })
}

fn parse_mintable(value: &BigInt) -> Result<bool, JettonMetadataError> {
    if value == &BigInt::from(-1) {
        Ok(true)
    } else if value == &BigInt::from(0) {
        Ok(false)
    } else {
        Err(JettonMetadataError::InvalidInteger {
            field: "mintable",
            reason: "expected -1 for true or 0 for false".to_string(),
        })
    }
}

fn parse_admin_address(cell: Arc<Cell>) -> Result<Option<Address>, JettonMetadataError> {
    let mut slice = Slice::new(cell);
    let address = MsgAddress::load_tlb(&mut slice)
        .map_err(|error| JettonMetadataError::MalformedAddress(error.to_string()))?;
    ensure_empty(&slice)
        .map_err(|error| JettonMetadataError::MalformedAddress(error.to_string()))?;
    match address {
        MsgAddress::Ext(MsgAddressExt::None) => Ok(None),
        MsgAddress::Int(MsgAddressInt::Std { anycast, address }) => {
            if anycast.is_some() {
                return Err(JettonMetadataError::MalformedAddress(
                    "admin address uses unsupported anycast".to_string(),
                ));
            }
            Ok(Some(address))
        }
        MsgAddress::Int(MsgAddressInt::Var { .. }) => Err(JettonMetadataError::MalformedAddress(
            "admin address uses unsupported variable-length address".to_string(),
        )),
        MsgAddress::Ext(MsgAddressExt::Extern { .. }) => {
            Err(JettonMetadataError::MalformedAddress(
                "admin address uses external-address constructor".to_string(),
            ))
        }
    }
}

fn stack_entry_int<'a>(
    stack: &'a [TvmStackEntry],
    index: usize,
    _field: &'static str,
) -> Result<&'a BigInt, JettonMetadataError> {
    match &stack[index] {
        TvmStackEntry::Int(value) => Ok(value),
        entry => Err(JettonMetadataError::StackType {
            index,
            expected: "integer",
            actual: stack_entry_type(entry),
        }),
    }
}

fn stack_entry_cell(
    stack: &[TvmStackEntry],
    index: usize,
) -> Result<Arc<Cell>, JettonMetadataError> {
    match &stack[index] {
        TvmStackEntry::Cell(cell) => Ok(cell.clone()),
        entry => Err(JettonMetadataError::StackType {
            index,
            expected: "cell",
            actual: stack_entry_type(entry),
        }),
    }
}

fn stack_entry_cell_like(
    stack: &[TvmStackEntry],
    index: usize,
    expected: &'static str,
) -> Result<Arc<Cell>, JettonMetadataError> {
    match &stack[index] {
        TvmStackEntry::Slice(cell) | TvmStackEntry::Cell(cell) => Ok(cell.clone()),
        entry => Err(JettonMetadataError::StackType {
            index,
            expected,
            actual: stack_entry_type(entry),
        }),
    }
}

fn stack_entry_type(entry: &TvmStackEntry) -> &'static str {
    match entry {
        TvmStackEntry::Null => "null",
        TvmStackEntry::Int(_) => "integer",
        TvmStackEntry::Cell(_) => "cell",
        TvmStackEntry::Slice(_) => "slice",
        TvmStackEntry::Tuple(_) => "tuple",
        TvmStackEntry::List(_) => "list",
        TvmStackEntry::Unsupported(_) => "unsupported",
    }
}

fn empty_cell() -> Arc<Cell> {
    crate::tvm::Builder::new()
        .build()
        .expect("empty cells are valid")
}

#[cfg(feature = "liteclient")]
impl<'a, P: crate::contracts::ContractProvider + ?Sized> crate::contracts::Contract<'a, P> {
    /// Runs `get_jetton_data` at the provider's latest masterchain block and
    /// decodes the returned TEP-74 stack.
    pub async fn jetton_master_data_latest(
        &mut self,
    ) -> Result<JettonMasterData, crate::contracts::ContractError<P::Error>> {
        let stack = self
            .run_get_method_by_name_typed_latest("get_jetton_data", TvmStack::empty())
            .await?;
        decode_jetton_master_data(stack).map_err(crate::contracts::ContractError::decode)
    }

    /// Runs `get_jetton_data` and returns parsed jetton metadata fields.
    pub async fn jetton_metadata_latest(
        &mut self,
    ) -> Result<JettonMetadata, crate::contracts::ContractError<P::Error>> {
        Ok(self.jetton_master_data_latest().await?.metadata())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Tep64Value, tep64_key_hash};
    use crate::tlb::{MsgAddress, TlbSerialize};
    use crate::tvm::{BitKey, Builder, HashmapE};

    const ON_CHAIN_TAG: u8 = 0x00;
    const OFF_CHAIN_TAG: u8 = 0x01;
    const TEP64_DICT_KEY_BITS: usize = 256;

    fn empty() -> Arc<Cell> {
        Builder::new().build().unwrap()
    }

    fn address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn admin_cell(address: Address) -> Arc<Cell> {
        MsgAddress::Int(MsgAddressInt::std(address))
            .to_cell()
            .unwrap()
    }

    fn offchain_content(uri: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(OFF_CHAIN_TAG).unwrap();
        builder.store_snake_bytes(uri).unwrap();
        builder.build().unwrap()
    }

    fn value_cell(tag: u8, bytes: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(tag).unwrap();
        builder.store_snake_bytes(bytes).unwrap();
        builder.build().unwrap()
    }

    fn malformed_value_cell() -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(ON_CHAIN_TAG).unwrap();
        builder.store_bit(true).unwrap();
        builder.build().unwrap()
    }

    fn key(name: &str) -> BitKey {
        BitKey::from_bits(tep64_key_hash(name).to_vec(), TEP64_DICT_KEY_BITS).unwrap()
    }

    fn onchain_content(entries: &[(&str, Arc<Cell>)]) -> Arc<Cell> {
        let mut dict = HashmapE::new(TEP64_DICT_KEY_BITS);
        for (name, value) in entries {
            dict.insert_bit_key(key(name), value.clone()).unwrap();
        }
        let mut builder = Builder::new();
        builder.store_u8(ON_CHAIN_TAG).unwrap();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_ref(value.clone())?;
                Ok(())
            })
            .unwrap();
        builder.build().unwrap()
    }

    fn valid_stack(content: Arc<Cell>) -> Vec<TvmStackEntry> {
        vec![
            TvmStackEntry::int(1000),
            TvmStackEntry::int(-1),
            TvmStackEntry::Slice(admin_cell(address(0x11))),
            TvmStackEntry::Cell(content),
            TvmStackEntry::Cell(empty()),
        ]
    }

    #[test]
    fn decodes_valid_get_jetton_data_stack() {
        let data = decode_jetton_master_data(valid_stack(offchain_content(
            b"https://example.test/jetton.json",
        )))
        .unwrap();

        assert_eq!(data.total_supply, BigUint::from(1000u32));
        assert!(data.mintable);
        assert_eq!(data.admin_address, Some(address(0x11)));
        assert!(matches!(data.jetton_content, Tep64Content::OffChain { .. }));
        assert_eq!(
            data.metadata().uri.as_deref(),
            Some("https://example.test/jetton.json")
        );
    }

    #[test]
    fn rejects_wrong_stack_length_and_types() {
        assert!(matches!(
            decode_jetton_master_data(vec![]),
            Err(JettonMetadataError::StackLength {
                actual: 0,
                expected: 5
            })
        ));

        let mut stack = valid_stack(offchain_content(b"https://example.test"));
        stack[0] = TvmStackEntry::Cell(empty());
        assert!(matches!(
            decode_jetton_master_data(stack),
            Err(JettonMetadataError::StackType {
                index: 0,
                expected: "integer",
                actual: "cell"
            })
        ));
    }

    #[test]
    fn mintable_accepts_only_minus_one_and_zero() {
        let mut stack = valid_stack(offchain_content(b"https://example.test"));
        stack[1] = TvmStackEntry::int(0);
        assert!(!decode_jetton_master_data(stack).unwrap().mintable);

        let mut stack = valid_stack(offchain_content(b"https://example.test"));
        stack[1] = TvmStackEntry::int(1);
        assert!(matches!(
            decode_jetton_master_data(stack),
            Err(JettonMetadataError::InvalidInteger {
                field: "mintable",
                ..
            })
        ));
    }

    #[test]
    fn rejects_malformed_admin_address() {
        let mut broken = Builder::new();
        broken.store_bit(true).unwrap();
        let mut stack = valid_stack(offchain_content(b"https://example.test"));
        stack[2] = TvmStackEntry::Slice(broken.build().unwrap());

        assert!(matches!(
            decode_jetton_master_data(stack),
            Err(JettonMetadataError::MalformedAddress(_))
        ));
    }

    #[test]
    fn rejects_negative_total_supply() {
        let mut stack = valid_stack(offchain_content(b"https://example.test"));
        stack[0] = TvmStackEntry::int(-1);

        assert!(matches!(
            decode_jetton_master_data(stack),
            Err(JettonMetadataError::InvalidInteger {
                field: "total_supply",
                ..
            })
        ));
    }

    #[test]
    fn maps_onchain_metadata_fields_and_preserves_unknowns() {
        let content = onchain_content(&[
            ("name", value_cell(ON_CHAIN_TAG, b"Example Jetton")),
            ("symbol", value_cell(ON_CHAIN_TAG, b"JET")),
            ("decimals", value_cell(ON_CHAIN_TAG, b"9")),
            ("description", value_cell(ON_CHAIN_TAG, b"Test asset")),
            (
                "image",
                value_cell(ON_CHAIN_TAG, b"https://example.test/image.png"),
            ),
            ("image_data", value_cell(ON_CHAIN_TAG, b"<svg/>")),
            ("custom", value_cell(ON_CHAIN_TAG, b"custom-value")),
        ]);

        let metadata = decode_jetton_master_data(valid_stack(content))
            .unwrap()
            .metadata();

        assert_eq!(metadata.name.as_deref(), Some("Example Jetton"));
        assert_eq!(metadata.symbol.as_deref(), Some("JET"));
        assert_eq!(metadata.decimals, Some(9));
        assert_eq!(metadata.description.as_deref(), Some("Test asset"));
        assert_eq!(
            metadata.image.as_deref(),
            Some("https://example.test/image.png")
        );
        assert_eq!(metadata.image_data.as_deref(), Some(&b"<svg/>"[..]));
        assert_eq!(metadata.unknown_fields.len(), 1);
        assert_eq!(
            metadata.unknown_fields[0].key_hash,
            tep64_key_hash("custom")
        );
        assert_eq!(
            metadata.unknown_fields[0].value,
            Tep64Value::Snake(b"custom-value".to_vec())
        );
    }

    #[test]
    fn malformed_known_field_becomes_diagnostic() {
        let content = onchain_content(&[("name", malformed_value_cell())]);
        let metadata = decode_jetton_master_data(valid_stack(content))
            .unwrap()
            .metadata();

        assert_eq!(metadata.name, None);
        assert_eq!(metadata.field_diagnostics.len(), 1);
        assert_eq!(
            metadata.field_diagnostics[0].known_key,
            Some(Tep64KnownKey::Name)
        );
        assert!(metadata.field_diagnostics[0].error.contains("byte-aligned"));
    }

    #[test]
    fn malformed_tep64_content_is_decode_error() {
        let mut builder = Builder::new();
        builder.store_u8(OFF_CHAIN_TAG).unwrap();
        builder.store_bit(true).unwrap();

        assert!(matches!(
            decode_jetton_master_data(valid_stack(builder.build().unwrap())),
            Err(JettonMetadataError::MalformedContent(_))
        ));
    }

    #[cfg(feature = "liteclient")]
    mod provider_tests {
        use super::*;
        use crate::contracts::{Contract, ContractError, ContractProvider};
        use crate::liteclient::boc::{DecodedAccountState, SimpleAccount};
        use crate::tl::{
            BlockIdExt, Int256,
            common::{AccountId, ZeroStateIdExt},
            response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
        };
        use async_trait::async_trait;

        #[derive(Debug, Error)]
        #[error("mock jetton provider error")]
        struct MockProviderError;

        struct MockProvider {
            latest: BlockIdExt,
            account: Address,
            stack: TvmStack,
            method_calls: Vec<u64>,
            fail_run_method: bool,
            exit_code: i32,
        }

        #[async_trait]
        impl ContractProvider for MockProvider {
            type Error = MockProviderError;

            async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
                Ok(MasterchainInfo {
                    last: self.latest.clone(),
                    state_root_hash: Int256([1; 32]),
                    init: ZeroStateIdExt {
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
                unreachable!("jetton metadata helper must not fetch account state")
            }

            async fn get_account_state_typed(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<DecodedAccountState, Self::Error> {
                unreachable!("jetton metadata helper must not fetch account state")
            }

            async fn get_account_state_simple(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<SimpleAccount, Self::Error> {
                unreachable!("jetton metadata helper must not fetch account state")
            }

            async fn run_get_method(
                &mut self,
                mode: u32,
                block: BlockIdExt,
                account: Address,
                method_id: u64,
                stack: TvmStack,
            ) -> Result<RunMethodResult, Self::Error> {
                if self.fail_run_method {
                    return Err(MockProviderError);
                }
                assert_eq!(mode, 0);
                assert_eq!(block, self.latest);
                assert_eq!(account, self.account);
                assert_eq!(stack, TvmStack::empty());
                self.method_calls.push(method_id);
                Ok(RunMethodResult {
                    mode: (),
                    id: self.latest.clone(),
                    shardblk: self.latest.clone(),
                    shard_proof: None,
                    proof: None,
                    state_proof: None,
                    init_c7: None,
                    lib_extras: None,
                    exit_code: self.exit_code,
                    result: Some(self.stack.to_boc().unwrap()),
                })
            }

            async fn send_external_message_boc(
                &mut self,
                _body: Vec<u8>,
            ) -> Result<u32, Self::Error> {
                unreachable!("jetton metadata helper must not send messages")
            }

            async fn get_transactions(
                &mut self,
                _count: u32,
                _account: AccountId,
                _lt: u64,
                _hash: Int256,
            ) -> Result<TransactionList, Self::Error> {
                unreachable!("jetton metadata helper must not fetch transactions")
            }
        }

        fn block(seqno: i32) -> BlockIdExt {
            BlockIdExt {
                workchain: -1,
                shard: i64::MIN,
                seqno,
                root_hash: Int256([4; 32]),
                file_hash: Int256([5; 32]),
            }
        }

        fn mock_provider(exit_code: i32, fail_run_method: bool) -> MockProvider {
            let account = address(0xaa);
            MockProvider {
                latest: block(10),
                account,
                stack: TvmStack::new(valid_stack(offchain_content(b"https://example.test"))),
                method_calls: Vec::new(),
                fail_run_method,
                exit_code,
            }
        }

        #[tokio::test]
        async fn provider_helper_uses_latest_block_and_get_jetton_data_method_id() {
            let mut provider = mock_provider(0, false);
            let account = provider.account.clone();
            {
                let mut contract = Contract::new(&mut provider, account);
                let data = contract.jetton_master_data_latest().await.unwrap();
                assert_eq!(data.total_supply, BigUint::from(1000u32));
            }

            assert_eq!(
                provider.method_calls,
                vec![crate::utils::method_name_to_id("get_jetton_data")]
            );
        }

        #[tokio::test]
        async fn provider_helper_propagates_provider_errors_and_exit_codes() {
            let mut provider = mock_provider(0, true);
            let account = provider.account.clone();
            let mut contract = Contract::new(&mut provider, account);
            assert!(matches!(
                contract.jetton_master_data_latest().await,
                Err(ContractError::Provider(_))
            ));

            let mut provider = mock_provider(13, false);
            let account = provider.account.clone();
            let mut contract = Contract::new(&mut provider, account);
            assert!(matches!(
                contract.jetton_master_data_latest().await,
                Err(ContractError::NonZeroExitCode { exit_code: 13 })
            ));
        }
    }
}
