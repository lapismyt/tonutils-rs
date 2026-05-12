//! TEP-62 NFT metadata helpers.
//!
//! This module decodes the `get_collection_data()`, `get_nft_data()`, and
//! `get_nft_content()` stack layouts used by TEP-62 NFT contracts, then maps
//! full TEP-64 content into NFT-oriented metadata fields. Off-chain JSON
//! fetching, transfers, royalty helpers, and indexer integration are
//! intentionally outside this layer.

use crate::metadata::{MetadataError, Tep64Content, Tep64Field, Tep64KnownKey, Tep64Value};
use crate::tlb::{MsgAddress, MsgAddressExt, MsgAddressInt, TlbDeserialize, ensure_empty};
use crate::tvm::{Address, Cell, Slice, TvmStack, TvmStackEntry};
use num_bigint::{BigInt, BigUint};
use std::sync::Arc;
use thiserror::Error;

const COLLECTION_DATA_STACK_LEN: usize = 3;
const NFT_DATA_STACK_LEN: usize = 5;
const NFT_CONTENT_STACK_LEN: usize = 1;

/// Typed result of a TEP-62 collection `get_collection_data()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftCollectionData {
    /// Next item index reported by the collection.
    pub next_item_index: BigUint,
    /// Collection content in TEP-64 format.
    pub collection_content: Tep64Content,
    /// Standard internal collection owner address, or `None` for `addr_none`.
    pub owner_address: Option<Address>,
}

impl NftCollectionData {
    /// Maps the parsed collection TEP-64 content into NFT metadata fields.
    pub fn metadata(&self) -> NftMetadata {
        NftMetadata::from_content(self.collection_content.clone())
    }
}

/// Typed result of a TEP-62 item `get_nft_data()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftItemData {
    /// `true` when `init?` is any non-zero integer.
    pub initialized: bool,
    /// Numerical item index.
    pub index: BigUint,
    /// Standard internal collection address, or `None` for standalone items.
    pub collection_address: Option<Address>,
    /// Standard internal owner address, or `None` for `addr_none`.
    pub owner_address: Option<Address>,
    /// Individual item content cell returned by the item contract.
    pub individual_content: Arc<Cell>,
}

/// NFT metadata fields recognized by common TEP-64 keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftMetadata {
    pub uri: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub render_type: Option<String>,
    pub content_url: Option<String>,
    pub video: Option<String>,
    pub raw_content: Tep64Content,
    pub unknown_fields: Vec<NftMetadataUnknownField>,
    pub field_diagnostics: Vec<NftMetadataFieldDiagnostic>,
}

impl NftMetadata {
    /// Builds typed NFT metadata from already-parsed TEP-64 content.
    pub fn from_content(raw_content: Tep64Content) -> Self {
        let mut metadata = Self {
            uri: None,
            name: None,
            description: None,
            image: None,
            image_data: None,
            render_type: None,
            content_url: None,
            video: None,
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
            self.preserve_unknown(field);
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
            Tep64KnownKey::RenderType => {
                self.render_type = self.field_string(field);
            }
            Tep64KnownKey::ContentUrl => {
                self.content_url = self.field_string(field);
            }
            Tep64KnownKey::Video => {
                self.video = self.field_string(field);
            }
            Tep64KnownKey::Symbol | Tep64KnownKey::Decimals | Tep64KnownKey::AmountStyle => {
                self.preserve_unknown(field);
            }
        }
    }

    fn preserve_unknown(&mut self, field: &Tep64Field) {
        self.unknown_fields.push(NftMetadataUnknownField {
            key_hash: field.key_hash,
            raw: field.raw.clone(),
            value: field.value.clone(),
        });
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

    fn field_bytes(&mut self, field: &Tep64Field) -> Option<Vec<u8>> {
        match &field.value {
            Tep64Value::Snake(bytes) => Some(bytes.clone()),
            Tep64Value::Chunked { bytes, .. } => Some(bytes.clone()),
            Tep64Value::Unsupported { .. } => {
                self.field_diagnostics.push(NftMetadataFieldDiagnostic {
                    key_hash: field.key_hash,
                    known_key: field.known_key,
                    error: "unsupported TEP-64 field value encoding".to_string(),
                    raw: field.raw.clone(),
                });
                None
            }
            Tep64Value::Malformed { error } => {
                self.field_diagnostics.push(NftMetadataFieldDiagnostic {
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
                self.field_diagnostics.push(NftMetadataFieldDiagnostic {
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

/// Raw-preserved metadata field that is not mapped to an NFT typed field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftMetadataUnknownField {
    pub key_hash: [u8; 32],
    pub raw: Arc<Cell>,
    pub value: Tep64Value,
}

/// Field-level metadata diagnostic that does not invalidate full metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NftMetadataFieldDiagnostic {
    pub key_hash: [u8; 32],
    pub known_key: Option<Tep64KnownKey>,
    pub error: String,
    pub raw: Arc<Cell>,
}

/// Errors returned while decoding TEP-62 NFT data and metadata.
#[derive(Debug, Error)]
pub enum NftMetadataError {
    #[error("{method} stack has {actual} entries, expected {expected}")]
    StackLength {
        method: &'static str,
        actual: usize,
        expected: usize,
    },
    #[error("{method} stack entry {index} has type {actual}, expected {expected}")]
    StackType {
        method: &'static str,
        index: usize,
        expected: &'static str,
        actual: &'static str,
    },
    #[error("{method} stack integer {field} is invalid: {reason}")]
    InvalidInteger {
        method: &'static str,
        field: &'static str,
        reason: String,
    },
    #[error("{method} {field} is malformed: {reason}")]
    MalformedAddress {
        method: &'static str,
        field: &'static str,
        reason: String,
    },
    #[error("{method} content cell is malformed: {source}")]
    MalformedContent {
        method: &'static str,
        #[source]
        source: MetadataError,
    },
    #[error("NFT item belongs to a collection; call get_nft_content on the collection contract")]
    CollectionContentRequired,
}

/// Decodes a successful TEP-62 `get_collection_data()` stack.
pub fn decode_nft_collection_data(
    stack: Vec<TvmStackEntry>,
) -> Result<NftCollectionData, NftMetadataError> {
    const METHOD: &str = "get_collection_data";
    if stack.len() != COLLECTION_DATA_STACK_LEN {
        return Err(NftMetadataError::StackLength {
            method: METHOD,
            actual: stack.len(),
            expected: COLLECTION_DATA_STACK_LEN,
        });
    }

    let next_item_index = parse_non_negative_int(
        stack_entry_int(&stack, 0, METHOD)?,
        METHOD,
        "next_item_index",
    )?;
    let collection_content = crate::metadata::parse_tep64_content(stack_entry_cell(
        &stack, 1, METHOD,
    )?)
    .map_err(|source| NftMetadataError::MalformedContent {
        method: METHOD,
        source,
    })?;
    let owner_address = parse_optional_standard_address(
        stack_entry_cell_like(&stack, 2, METHOD, "slice/cell")?,
        METHOD,
        "owner_address",
    )?;

    Ok(NftCollectionData {
        next_item_index,
        collection_content,
        owner_address,
    })
}

/// Decodes a `TvmStack` containing `get_collection_data()` output.
pub fn decode_nft_collection_data_stack(
    stack: TvmStack,
) -> Result<NftCollectionData, NftMetadataError> {
    decode_nft_collection_data(stack.entries().to_vec())
}

/// Decodes a successful TEP-62 `get_nft_data()` stack.
pub fn decode_nft_item_data(stack: Vec<TvmStackEntry>) -> Result<NftItemData, NftMetadataError> {
    const METHOD: &str = "get_nft_data";
    if stack.len() != NFT_DATA_STACK_LEN {
        return Err(NftMetadataError::StackLength {
            method: METHOD,
            actual: stack.len(),
            expected: NFT_DATA_STACK_LEN,
        });
    }

    let initialized = stack_entry_int(&stack, 0, METHOD)? != &BigInt::from(0);
    let index = parse_non_negative_int(stack_entry_int(&stack, 1, METHOD)?, METHOD, "index")?;
    let collection_address = parse_optional_standard_address(
        stack_entry_cell_like(&stack, 2, METHOD, "slice/cell")?,
        METHOD,
        "collection_address",
    )?;
    let owner_address = parse_optional_standard_address(
        stack_entry_cell_like(&stack, 3, METHOD, "slice/cell")?,
        METHOD,
        "owner_address",
    )?;
    let individual_content = stack_entry_cell(&stack, 4, METHOD)?;

    Ok(NftItemData {
        initialized,
        index,
        collection_address,
        owner_address,
        individual_content,
    })
}

/// Decodes a `TvmStack` containing `get_nft_data()` output.
pub fn decode_nft_item_data_stack(stack: TvmStack) -> Result<NftItemData, NftMetadataError> {
    decode_nft_item_data(stack.entries().to_vec())
}

/// Decodes a successful TEP-62 `get_nft_content()` stack and maps it to NFT metadata.
pub fn decode_nft_full_content_metadata(
    stack: Vec<TvmStackEntry>,
) -> Result<NftMetadata, NftMetadataError> {
    const METHOD: &str = "get_nft_content";
    if stack.len() != NFT_CONTENT_STACK_LEN {
        return Err(NftMetadataError::StackLength {
            method: METHOD,
            actual: stack.len(),
            expected: NFT_CONTENT_STACK_LEN,
        });
    }

    parse_nft_metadata_cell(stack_entry_cell(&stack, 0, METHOD)?)
}

/// Parses a TEP-64 full content cell into NFT metadata fields.
pub fn parse_nft_metadata_cell(cell: Arc<Cell>) -> Result<NftMetadata, NftMetadataError> {
    crate::metadata::parse_tep64_content(cell)
        .map(NftMetadata::from_content)
        .map_err(|source| NftMetadataError::MalformedContent {
            method: "TEP-64",
            source,
        })
}

fn parse_non_negative_int(
    value: &BigInt,
    method: &'static str,
    field: &'static str,
) -> Result<BigUint, NftMetadataError> {
    value
        .to_biguint()
        .ok_or_else(|| NftMetadataError::InvalidInteger {
            method,
            field,
            reason: "value must be non-negative".to_string(),
        })
}

fn parse_optional_standard_address(
    cell: Arc<Cell>,
    method: &'static str,
    field: &'static str,
) -> Result<Option<Address>, NftMetadataError> {
    let mut slice = Slice::new(cell);
    let address =
        MsgAddress::load_tlb(&mut slice).map_err(|error| NftMetadataError::MalformedAddress {
            method,
            field,
            reason: error.to_string(),
        })?;
    ensure_empty(&slice).map_err(|error| NftMetadataError::MalformedAddress {
        method,
        field,
        reason: error.to_string(),
    })?;
    match address {
        MsgAddress::Ext(MsgAddressExt::None) => Ok(None),
        MsgAddress::Int(MsgAddressInt::Std { anycast, address }) => {
            if anycast.is_some() {
                return Err(NftMetadataError::MalformedAddress {
                    method,
                    field,
                    reason: format!("{field} uses unsupported anycast"),
                });
            }
            Ok(Some(address))
        }
        MsgAddress::Int(MsgAddressInt::Var { .. }) => Err(NftMetadataError::MalformedAddress {
            method,
            field,
            reason: format!("{field} uses unsupported variable-length address"),
        }),
        MsgAddress::Ext(MsgAddressExt::Extern { .. }) => Err(NftMetadataError::MalformedAddress {
            method,
            field,
            reason: format!("{field} uses external-address constructor"),
        }),
    }
}

fn stack_entry_int<'a>(
    stack: &'a [TvmStackEntry],
    index: usize,
    method: &'static str,
) -> Result<&'a BigInt, NftMetadataError> {
    match &stack[index] {
        TvmStackEntry::Int(value) => Ok(value),
        entry => Err(NftMetadataError::StackType {
            method,
            index,
            expected: "integer",
            actual: stack_entry_type(entry),
        }),
    }
}

fn stack_entry_cell(
    stack: &[TvmStackEntry],
    index: usize,
    method: &'static str,
) -> Result<Arc<Cell>, NftMetadataError> {
    match &stack[index] {
        TvmStackEntry::Cell(cell) => Ok(cell.clone()),
        entry => Err(NftMetadataError::StackType {
            method,
            index,
            expected: "cell",
            actual: stack_entry_type(entry),
        }),
    }
}

fn stack_entry_cell_like(
    stack: &[TvmStackEntry],
    index: usize,
    method: &'static str,
    expected: &'static str,
) -> Result<Arc<Cell>, NftMetadataError> {
    match &stack[index] {
        TvmStackEntry::Slice(cell) | TvmStackEntry::Cell(cell) => Ok(cell.clone()),
        entry => Err(NftMetadataError::StackType {
            method,
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
    /// Runs `get_collection_data` at the provider's latest masterchain block.
    pub async fn nft_collection_data_latest(
        &mut self,
    ) -> Result<NftCollectionData, crate::contracts::ContractError<P::Error>> {
        let stack = self
            .run_get_method_by_name_typed_latest("get_collection_data", TvmStack::empty())
            .await?;
        decode_nft_collection_data(stack).map_err(crate::contracts::ContractError::decode)
    }

    /// Runs `get_nft_data` at the provider's latest masterchain block.
    pub async fn nft_item_data_latest(
        &mut self,
    ) -> Result<NftItemData, crate::contracts::ContractError<P::Error>> {
        let stack = self
            .run_get_method_by_name_typed_latest("get_nft_data", TvmStack::empty())
            .await?;
        decode_nft_item_data(stack).map_err(crate::contracts::ContractError::decode)
    }

    /// Runs `get_collection_data` and returns parsed collection metadata.
    pub async fn nft_collection_metadata_latest(
        &mut self,
    ) -> Result<NftMetadata, crate::contracts::ContractError<P::Error>> {
        Ok(self.nft_collection_data_latest().await?.metadata())
    }

    /// Runs `get_nft_data` and parses item content directly for standalone NFTs.
    ///
    /// Collection-backed items normally return partial individual content here.
    /// Use `nft_full_item_metadata_latest` on the collection contract to run
    /// `get_nft_content(index, individual_content)` and parse the returned full
    /// TEP-64 content.
    pub async fn nft_item_metadata_latest(
        &mut self,
    ) -> Result<NftMetadata, crate::contracts::ContractError<P::Error>> {
        let data = self.nft_item_data_latest().await?;
        if data.collection_address.is_some() {
            return Err(crate::contracts::ContractError::decode(
                NftMetadataError::CollectionContentRequired,
            ));
        }
        parse_nft_metadata_cell(data.individual_content)
            .map_err(crate::contracts::ContractError::decode)
    }

    /// Runs collection `get_nft_content(index, individual_content)` and parses
    /// the returned full TEP-64 metadata.
    pub async fn nft_full_item_metadata_latest(
        &mut self,
        item_data: &NftItemData,
    ) -> Result<NftMetadata, crate::contracts::ContractError<P::Error>> {
        let stack = TvmStack::new(vec![
            TvmStackEntry::Int(BigInt::from(item_data.index.clone())),
            TvmStackEntry::Cell(item_data.individual_content.clone()),
        ]);
        let full_content = self
            .run_get_method_by_name_typed_latest("get_nft_content", stack)
            .await?;
        decode_nft_full_content_metadata(full_content)
            .map_err(crate::contracts::ContractError::decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Tep64Value, tep64_key_hash};
    use crate::tlb::{Anycast, MsgAddress, TlbSerialize};
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

    fn address_cell(address: Address) -> Arc<Cell> {
        MsgAddress::Int(MsgAddressInt::std(address))
            .to_cell()
            .unwrap()
    }

    fn none_address_cell() -> Arc<Cell> {
        MsgAddress::Ext(MsgAddressExt::None).to_cell().unwrap()
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

    fn collection_stack(content: Arc<Cell>) -> Vec<TvmStackEntry> {
        vec![
            TvmStackEntry::int(10),
            TvmStackEntry::Cell(content),
            TvmStackEntry::Slice(address_cell(address(0x11))),
        ]
    }

    fn item_stack(content: Arc<Cell>) -> Vec<TvmStackEntry> {
        vec![
            TvmStackEntry::int(-1),
            TvmStackEntry::int(7),
            TvmStackEntry::Slice(address_cell(address(0x22))),
            TvmStackEntry::Slice(address_cell(address(0x33))),
            TvmStackEntry::Cell(content),
        ]
    }

    #[test]
    fn decodes_valid_collection_data_stack() {
        let data = decode_nft_collection_data(collection_stack(offchain_content(
            b"https://example.test/collection.json",
        )))
        .unwrap();

        assert_eq!(data.next_item_index, BigUint::from(10u32));
        assert_eq!(data.owner_address, Some(address(0x11)));
        assert_eq!(
            data.metadata().uri.as_deref(),
            Some("https://example.test/collection.json")
        );
    }

    #[test]
    fn decodes_valid_item_data_stack() {
        let data = decode_nft_item_data(item_stack(empty())).unwrap();

        assert!(data.initialized);
        assert_eq!(data.index, BigUint::from(7u32));
        assert_eq!(data.collection_address, Some(address(0x22)));
        assert_eq!(data.owner_address, Some(address(0x33)));
    }

    #[test]
    fn rejects_wrong_stack_length_and_types() {
        assert!(matches!(
            decode_nft_collection_data(vec![]),
            Err(NftMetadataError::StackLength {
                method: "get_collection_data",
                actual: 0,
                expected: 3
            })
        ));

        let mut stack = item_stack(empty());
        stack[1] = TvmStackEntry::Cell(empty());
        assert!(matches!(
            decode_nft_item_data(stack),
            Err(NftMetadataError::StackType {
                method: "get_nft_data",
                index: 1,
                expected: "integer",
                actual: "cell"
            })
        ));
    }

    #[test]
    fn rejects_negative_indices() {
        let mut stack = collection_stack(offchain_content(b"https://example.test"));
        stack[0] = TvmStackEntry::int(-1);
        assert!(matches!(
            decode_nft_collection_data(stack),
            Err(NftMetadataError::InvalidInteger {
                method: "get_collection_data",
                field: "next_item_index",
                ..
            })
        ));

        let mut stack = item_stack(empty());
        stack[1] = TvmStackEntry::int(-1);
        assert!(matches!(
            decode_nft_item_data(stack),
            Err(NftMetadataError::InvalidInteger {
                method: "get_nft_data",
                field: "index",
                ..
            })
        ));
    }

    #[test]
    fn init_maps_zero_to_false_and_non_zero_to_true() {
        let mut stack = item_stack(empty());
        stack[0] = TvmStackEntry::int(0);
        assert!(!decode_nft_item_data(stack).unwrap().initialized);

        let mut stack = item_stack(empty());
        stack[0] = TvmStackEntry::int(42);
        assert!(decode_nft_item_data(stack).unwrap().initialized);
    }

    #[test]
    fn accepts_addr_none_as_none() {
        let mut stack = item_stack(empty());
        stack[2] = TvmStackEntry::Slice(none_address_cell());
        stack[3] = TvmStackEntry::Slice(none_address_cell());

        let data = decode_nft_item_data(stack).unwrap();

        assert_eq!(data.collection_address, None);
        assert_eq!(data.owner_address, None);
    }

    #[test]
    fn rejects_malformed_trailing_external_var_and_anycast_addresses() {
        let malformed = {
            let mut builder = Builder::new();
            builder.store_bit(true).unwrap();
            builder.build().unwrap()
        };
        let trailing = {
            let mut builder = Builder::new();
            builder
                .store_slice(&Slice::new(address_cell(address(0x44))))
                .unwrap();
            builder.store_bit(true).unwrap();
            builder.build().unwrap()
        };
        let external = MsgAddress::Ext(MsgAddressExt::Extern {
            data: vec![0x80],
            bit_len: 1,
        })
        .to_cell()
        .unwrap();
        let var = MsgAddress::Int(MsgAddressInt::Var {
            anycast: None,
            workchain_id: 0,
            address: vec![0x80],
            bit_len: 1,
        })
        .to_cell()
        .unwrap();
        let anycast = MsgAddress::Int(MsgAddressInt::Std {
            anycast: Some(Anycast {
                depth: 1,
                rewrite_pfx: vec![0x80],
            }),
            address: address(0x55),
        })
        .to_cell()
        .unwrap();

        for cell in [malformed, trailing, external, var, anycast] {
            let mut stack = item_stack(empty());
            stack[2] = TvmStackEntry::Slice(cell);
            assert!(matches!(
                decode_nft_item_data(stack),
                Err(NftMetadataError::MalformedAddress {
                    method: "get_nft_data",
                    field: "collection_address",
                    ..
                })
            ));
        }
    }

    #[test]
    fn maps_full_nft_metadata_fields_and_preserves_unknowns() {
        let content = onchain_content(&[
            ("name", value_cell(ON_CHAIN_TAG, b"Example NFT")),
            ("description", value_cell(ON_CHAIN_TAG, b"Test item")),
            (
                "image",
                value_cell(ON_CHAIN_TAG, b"https://example.test/image.png"),
            ),
            ("image_data", value_cell(ON_CHAIN_TAG, b"<svg/>")),
            ("render_type", value_cell(ON_CHAIN_TAG, b"game")),
            (
                "content_url",
                value_cell(ON_CHAIN_TAG, b"https://example.test/content"),
            ),
            (
                "video",
                value_cell(ON_CHAIN_TAG, b"https://example.test/video.mp4"),
            ),
            ("symbol", value_cell(ON_CHAIN_TAG, b"NFT")),
            ("custom", value_cell(ON_CHAIN_TAG, b"custom-value")),
        ]);

        let metadata = parse_nft_metadata_cell(content).unwrap();

        assert_eq!(metadata.name.as_deref(), Some("Example NFT"));
        assert_eq!(metadata.description.as_deref(), Some("Test item"));
        assert_eq!(
            metadata.image.as_deref(),
            Some("https://example.test/image.png")
        );
        assert_eq!(metadata.image_data.as_deref(), Some(&b"<svg/>"[..]));
        assert_eq!(metadata.render_type.as_deref(), Some("game"));
        assert_eq!(
            metadata.content_url.as_deref(),
            Some("https://example.test/content")
        );
        assert_eq!(
            metadata.video.as_deref(),
            Some("https://example.test/video.mp4")
        );
        assert_eq!(metadata.unknown_fields.len(), 2);
        assert!(
            metadata
                .unknown_fields
                .iter()
                .any(|field| field.key_hash == tep64_key_hash("symbol"))
        );
        let custom = metadata
            .unknown_fields
            .iter()
            .find(|field| field.key_hash == tep64_key_hash("custom"))
            .unwrap();
        assert_eq!(custom.value, Tep64Value::Snake(b"custom-value".to_vec()));
    }

    #[test]
    fn malformed_known_field_becomes_diagnostic() {
        let metadata =
            parse_nft_metadata_cell(onchain_content(&[("name", malformed_value_cell())])).unwrap();

        assert_eq!(metadata.name, None);
        assert_eq!(metadata.field_diagnostics.len(), 1);
        assert_eq!(
            metadata.field_diagnostics[0].known_key,
            Some(Tep64KnownKey::Name)
        );
        assert!(metadata.field_diagnostics[0].error.contains("byte-aligned"));
    }

    #[test]
    fn decodes_get_nft_content_stack() {
        let metadata = decode_nft_full_content_metadata(vec![TvmStackEntry::Cell(
            offchain_content(b"https://example.test/item.json"),
        )])
        .unwrap();

        assert_eq!(
            metadata.uri.as_deref(),
            Some("https://example.test/item.json")
        );
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
        #[error("mock nft provider error")]
        struct MockProviderError;

        struct ExpectedCall {
            account: Address,
            method_id: u64,
            stack: TvmStack,
            result: TvmStack,
            exit_code: i32,
        }

        struct MockProvider {
            latest: BlockIdExt,
            calls: Vec<ExpectedCall>,
            seen_methods: Vec<u64>,
            fail_run_method: bool,
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
                unreachable!("nft metadata helper must not fetch account state")
            }

            async fn get_account_state_typed(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<DecodedAccountState, Self::Error> {
                unreachable!("nft metadata helper must not fetch account state")
            }

            async fn get_account_state_simple(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<SimpleAccount, Self::Error> {
                unreachable!("nft metadata helper must not fetch account state")
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
                let expected = self.calls.remove(0);
                assert_eq!(account, expected.account);
                assert_eq!(method_id, expected.method_id);
                assert_eq!(stack, expected.stack);
                self.seen_methods.push(method_id);
                Ok(RunMethodResult {
                    mode: (),
                    id: self.latest.clone(),
                    shardblk: self.latest.clone(),
                    shard_proof: None,
                    proof: None,
                    state_proof: None,
                    init_c7: None,
                    lib_extras: None,
                    exit_code: expected.exit_code,
                    result: Some(expected.result.to_boc().unwrap()),
                })
            }

            async fn send_external_message_boc(
                &mut self,
                _body: Vec<u8>,
            ) -> Result<u32, Self::Error> {
                unreachable!("nft metadata helper must not send messages")
            }

            async fn get_transactions(
                &mut self,
                _count: u32,
                _account: AccountId,
                _lt: u64,
                _hash: Int256,
            ) -> Result<TransactionList, Self::Error> {
                unreachable!("nft metadata helper must not fetch transactions")
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

        fn mock_provider(calls: Vec<ExpectedCall>) -> MockProvider {
            MockProvider {
                latest: block(10),
                calls,
                seen_methods: Vec::new(),
                fail_run_method: false,
            }
        }

        fn call(
            account: Address,
            method: &str,
            stack: TvmStack,
            result: TvmStack,
            exit_code: i32,
        ) -> ExpectedCall {
            ExpectedCall {
                account,
                method_id: crate::utils::method_name_to_id(method),
                stack,
                result,
                exit_code,
            }
        }

        #[tokio::test]
        async fn collection_helper_uses_latest_block_empty_stack_and_method_id() {
            let account = address(0xaa);
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_collection_data",
                TvmStack::empty(),
                TvmStack::new(collection_stack(offchain_content(b"https://example.test"))),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let data = contract.nft_collection_data_latest().await.unwrap();

            assert_eq!(data.next_item_index, BigUint::from(10u32));
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_collection_data")]
            );
        }

        #[tokio::test]
        async fn item_helper_uses_latest_block_empty_stack_and_method_id() {
            let account = address(0xab);
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::new(item_stack(offchain_content(
                    b"https://example.test/item.json",
                ))),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let data = contract.nft_item_data_latest().await.unwrap();

            assert_eq!(data.index, BigUint::from(7u32));
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_nft_data")]
            );
        }

        #[tokio::test]
        async fn full_item_helper_passes_index_and_individual_content() {
            let collection = address(0xac);
            let individual_content = offchain_content(b"7.json");
            let item_data = NftItemData {
                initialized: true,
                index: BigUint::from(7u32),
                collection_address: Some(collection.clone()),
                owner_address: Some(address(0xad)),
                individual_content: individual_content.clone(),
            };
            let expected_stack = TvmStack::new(vec![
                TvmStackEntry::Int(BigInt::from(7u32)),
                TvmStackEntry::Cell(individual_content),
            ]);
            let mut provider = mock_provider(vec![call(
                collection.clone(),
                "get_nft_content",
                expected_stack,
                TvmStack::new(vec![TvmStackEntry::Cell(offchain_content(
                    b"https://example.test/item/7.json",
                ))]),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, collection);
            let metadata = contract
                .nft_full_item_metadata_latest(&item_data)
                .await
                .unwrap();

            assert_eq!(
                metadata.uri.as_deref(),
                Some("https://example.test/item/7.json")
            );
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_nft_content")]
            );
        }

        #[tokio::test]
        async fn standalone_item_metadata_parses_individual_content_directly() {
            let account = address(0xae);
            let mut stack = item_stack(offchain_content(b"https://example.test/standalone.json"));
            stack[2] = TvmStackEntry::Slice(none_address_cell());
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::new(stack),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let metadata = contract.nft_item_metadata_latest().await.unwrap();

            assert_eq!(
                metadata.uri.as_deref(),
                Some("https://example.test/standalone.json")
            );
        }

        #[tokio::test]
        async fn provider_helper_propagates_provider_errors_and_exit_codes() {
            let account = address(0xaf);
            let mut provider = mock_provider(Vec::new());
            provider.fail_run_method = true;
            let mut contract = Contract::new(&mut provider, account.clone());
            assert!(matches!(
                contract.nft_item_data_latest().await,
                Err(ContractError::Provider(_))
            ));

            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::empty(),
                13,
            )]);
            let mut contract = Contract::new(&mut provider, account);
            assert!(matches!(
                contract.nft_item_data_latest().await,
                Err(ContractError::NonZeroExitCode { exit_code: 13 })
            ));
        }
    }
}
