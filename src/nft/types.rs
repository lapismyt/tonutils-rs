use crate::metadata::{MetadataError, Tep64Content, Tep64Field, Tep64KnownKey, Tep64Value};
use crate::tlb::{MsgAddress, MsgAddressExt, MsgAddressInt, TlbDeserialize, ensure_empty};
use crate::tvm::{Address, Cell, Slice, TvmStack, TvmStackEntry};
use num_bigint::{BigInt, BigUint};
use std::sync::Arc;
use thiserror::Error;

pub(super) const COLLECTION_DATA_STACK_LEN: usize = 3;
pub(super) const NFT_DATA_STACK_LEN: usize = 5;
pub(super) const NFT_CONTENT_STACK_LEN: usize = 1;

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

pub(super) fn parse_non_negative_int(
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

pub(super) fn parse_optional_standard_address(
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

pub(super) fn stack_entry_int<'a>(
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

pub(super) fn stack_entry_cell(
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

pub(super) fn stack_entry_cell_like(
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

pub(super) fn stack_entry_type(entry: &TvmStackEntry) -> &'static str {
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

pub(super) fn empty_cell() -> Arc<Cell> {
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
