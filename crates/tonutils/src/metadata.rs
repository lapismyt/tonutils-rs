//! TEP-64 metadata content parsing helpers.
//!
//! The module decodes the common metadata cell layouts used by jettons and NFTs
//! while preserving raw cells for unknown keys and unsupported future formats.

use std::sync::Arc;

use crate::tvm::{BitKey, Builder, Cell, HashmapE, Slice, TvmError};

use sha2::{Digest, Sha256};
use thiserror::Error;

const TEP64_ON_CHAIN_TAG: u8 = 0x00;
const TEP64_OFF_CHAIN_TAG: u8 = 0x01;
const TEP64_DICT_KEY_BITS: usize = 256;
const TEP64_CHUNK_KEY_BITS: usize = 32;
const TEP64_CHUNK_BYTES: usize = 127;

/// Error returned when a TEP-64 content cell is malformed.
#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("metadata snake cell has {bits} trailing bits, expected byte-aligned data")]
    NonByteAlignedSnake { bits: usize },
    #[error("metadata snake cell has {refs} continuation references, expected at most 1")]
    TooManySnakeRefs { refs: usize },
    #[error("metadata snake continuation is malformed: {0}")]
    MalformedSnake(String),
    #[error("metadata on-chain dictionary is malformed: {0}")]
    MalformedOnChainDictionary(String),
    #[error("metadata chunked dictionary is malformed: {0}")]
    MalformedChunkedDictionary(String),
    #[error("failed to encode metadata cell: {0}")]
    Encode(#[from] TvmError),
    #[error("metadata field {key} cannot be serialized: {reason}")]
    UnserializableField {
        /// Field name or hex key.
        key: String,
        /// Why serialization is not possible.
        reason: String,
    },
    #[error("on-chain metadata requires the TEP-64 prefix byte")]
    MissingOnChainPrefix,
    #[error("duplicate TEP-64 field key: {0}")]
    DuplicateField(String),
}

/// Parsed top-level TEP-64 content.
///
/// Equality compares the decoded content only. The preserved raw cell is an
/// encoding cache: the same off-chain URI can be stored with or without the
/// TEP-64 prefix depending on the contract edition.
#[derive(Debug, Clone, Eq)]
pub enum Tep64Content {
    /// `0x01` followed by a snake-encoded off-chain URI.
    OffChain {
        /// Decoded URI bytes.
        uri: Vec<u8>,
        /// Original content cell.
        raw: Arc<Cell>,
    },
    /// `0x00` followed by `HashmapE 256 ^Cell` keyed by SHA-256 field names.
    OnChain {
        /// Decoded field entries in canonical dictionary order.
        fields: Vec<Tep64Field>,
        /// Original content cell.
        raw: Arc<Cell>,
    },
    /// Unknown or too-short top-level marker. The raw cell is preserved.
    Unsupported {
        /// First byte when the cell has one.
        tag: Option<u8>,
        /// Original content cell.
        raw: Arc<Cell>,
    },
}

impl PartialEq for Tep64Content {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::OffChain { uri: left, .. }, Self::OffChain { uri: right, .. }) => left == right,
            (Self::OnChain { fields: left, .. }, Self::OnChain { fields: right, .. }) => {
                left == right
            }
            (Self::Unsupported { tag: left, .. }, Self::Unsupported { tag: right, .. }) => {
                left == right
            }
            _ => false,
        }
    }
}

/// One on-chain metadata dictionary entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tep64Field {
    /// 256-bit SHA-256 dictionary key.
    pub key_hash: [u8; 32],
    /// Known TEP-64 key name, when recognized.
    pub known_key: Option<Tep64KnownKey>,
    /// Parsed or raw-preserved value.
    pub value: Tep64Value,
    /// Original value cell referenced by the dictionary.
    pub raw: Arc<Cell>,
}

/// Common TEP-64 keys used by jetton and NFT metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tep64KnownKey {
    Uri,
    Name,
    Description,
    Image,
    ImageData,
    Symbol,
    Decimals,
    AmountStyle,
    RenderType,
    ContentUrl,
    Video,
}

impl Tep64KnownKey {
    /// Returns the canonical string stored before SHA-256 key hashing.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uri => "uri",
            Self::Name => "name",
            Self::Description => "description",
            Self::Image => "image",
            Self::ImageData => "image_data",
            Self::Symbol => "symbol",
            Self::Decimals => "decimals",
            Self::AmountStyle => "amount_style",
            Self::RenderType => "render_type",
            Self::ContentUrl => "content_url",
            Self::Video => "video",
        }
    }

    /// Returns the SHA-256 dictionary key used by on-chain TEP-64 metadata.
    pub fn key_hash(self) -> [u8; 32] {
        tep64_key_hash(self.as_str())
    }
}

/// Parsed value of an on-chain TEP-64 dictionary field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tep64Value {
    /// Snake-encoded bytes, with the optional in-value `0x00` marker removed.
    Snake(Vec<u8>),
    /// Chunked bytes concatenated in ascending 32-bit chunk-index order.
    Chunked {
        /// Individual decoded chunks.
        chunks: Vec<Tep64Chunk>,
        /// Concatenated chunk bytes.
        bytes: Vec<u8>,
    },
    /// Unsupported future value marker; raw bytes/cell are preserved.
    Unsupported {
        /// First byte of the raw value, when present.
        tag: Option<u8>,
        /// Raw flattened bytes when byte-aligned.
        bytes: Option<Vec<u8>>,
    },
    /// Malformed value kept as a field-level diagnostic instead of dropping the
    /// containing on-chain dictionary.
    Malformed {
        /// Decode error.
        error: String,
    },
}

/// One decoded chunk from chunked TEP-64 data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tep64Chunk {
    /// 32-bit chunk index.
    pub index: u32,
    /// Decoded snake bytes for this chunk.
    pub bytes: Vec<u8>,
}

impl Tep64Content {
    /// Builds off-chain content from a URI.
    ///
    /// The returned value keeps a canonical prefixed cell in [`Self::OffChain::raw`].
    pub fn off_chain(uri: impl Into<Vec<u8>>) -> Result<Self, MetadataError> {
        let uri = uri.into();
        let mut builder = Builder::new();
        builder.store_u8(TEP64_OFF_CHAIN_TAG)?;
        builder.store_snake_bytes(&uri)?;
        Ok(Self::OffChain {
            uri,
            raw: builder.build()?,
        })
    }

    /// Builds on-chain content from dictionary fields.
    pub fn on_chain(fields: Vec<Tep64Field>) -> Result<Self, MetadataError> {
        let content = Self::OnChain {
            fields,
            raw: Builder::new().build()?,
        };
        let raw = content.to_cell_with_prefix(true)?;
        match content {
            Self::OnChain { fields, .. } => Ok(Self::OnChain { fields, raw }),
            _ => unreachable!("constructed on-chain content"),
        }
    }

    /// Returns the off-chain URI, when this is off-chain content.
    pub fn off_chain_uri(&self) -> Option<&[u8]> {
        match self {
            Self::OffChain { uri, .. } => Some(uri),
            _ => None,
        }
    }

    /// Returns on-chain fields, when this is on-chain content.
    pub fn on_chain_fields(&self) -> Option<&[Tep64Field]> {
        match self {
            Self::OnChain { fields, .. } => Some(fields),
            _ => None,
        }
    }

    /// Encodes the content with the TEP-64 prefix byte.
    pub fn to_cell(&self) -> Result<Arc<Cell>, MetadataError> {
        self.to_cell_with_prefix(true)
    }

    /// Encodes the content, optionally omitting the TEP-64 prefix byte.
    ///
    /// Stablecoin jetton masters store their off-chain URI as a bare snake
    /// string, which is the `with_prefix = false` form.
    pub fn to_cell_with_prefix(&self, with_prefix: bool) -> Result<Arc<Cell>, MetadataError> {
        match self {
            Self::OffChain { uri, .. } => {
                let mut builder = Builder::new();
                if with_prefix {
                    builder.store_u8(TEP64_OFF_CHAIN_TAG)?;
                }
                builder.store_snake_bytes(uri)?;
                Ok(builder.build()?)
            }
            Self::OnChain { fields, .. } => {
                if !with_prefix {
                    return Err(MetadataError::MissingOnChainPrefix);
                }
                let mut dict: HashmapE<Arc<Cell>> = HashmapE::new(TEP64_DICT_KEY_BITS);
                for field in fields {
                    let key = BitKey::from_bits(field.key_hash.to_vec(), TEP64_DICT_KEY_BITS)
                        .map_err(MetadataError::Encode)?;
                    if dict
                        .insert_bit_key(key, field.raw.clone())
                        .map_err(MetadataError::Encode)?
                        .is_some()
                    {
                        return Err(MetadataError::DuplicateField(field.known_key.map_or_else(
                            || "unknown on-chain field".to_string(),
                            |key| key.as_str().to_string(),
                        )));
                    }
                }
                let mut builder = Builder::new();
                builder.store_u8(TEP64_ON_CHAIN_TAG)?;
                builder.store_hashmap_e_with(&dict, |builder, value| {
                    builder.store_ref(value.clone())?;
                    Ok(())
                })?;
                Ok(builder.build()?)
            }
            Self::Unsupported { .. } => Err(MetadataError::UnserializableField {
                key: "content".to_string(),
                reason: "unsupported content cannot be re-encoded".to_string(),
            }),
        }
    }
}

impl Tep64Field {
    /// Creates a snake-encoded field.
    pub fn snake(name: &str, value: impl AsRef<[u8]>) -> Result<Self, MetadataError> {
        let bytes = value.as_ref();
        let mut builder = Builder::new();
        builder.store_u8(TEP64_ON_CHAIN_TAG)?;
        builder.store_snake_bytes(bytes)?;
        Ok(Self {
            key_hash: tep64_key_hash(name),
            known_key: known_key_by_hash(tep64_key_hash(name)),
            value: Tep64Value::Snake(bytes.to_vec()),
            raw: builder.build()?,
        })
    }

    /// Creates a chunked field, splitting `value` into 32-bit keyed chunks.
    pub fn chunked(name: &str, value: impl AsRef<[u8]>) -> Result<Self, MetadataError> {
        let bytes = value.as_ref();
        let chunks = encode_chunks(bytes)?;
        let mut dict: HashmapE<Arc<Cell>> = HashmapE::new(TEP64_CHUNK_KEY_BITS);
        for chunk in &chunks {
            let key = BitKey::from_u64(u64::from(chunk.index), TEP64_CHUNK_KEY_BITS)
                .map_err(MetadataError::Encode)?;
            let mut chunk_builder = Builder::new();
            chunk_builder.store_snake_bytes(&chunk.bytes)?;
            dict.insert_bit_key(key, chunk_builder.build()?)
                .map_err(MetadataError::Encode)?;
        }
        let mut builder = Builder::new();
        builder.store_u8(TEP64_OFF_CHAIN_TAG)?;
        builder.store_hashmap_e_with(&dict, |builder, cell| {
            builder.store_ref(cell.clone())?;
            Ok(())
        })?;
        let key_hash = tep64_key_hash(name);
        Ok(Self {
            key_hash,
            known_key: known_key_by_hash(key_hash),
            value: Tep64Value::Chunked {
                chunks,
                bytes: bytes.to_vec(),
            },
            raw: builder.build()?,
        })
    }

    /// Creates a field from an already-encoded value cell.
    pub fn raw_cell(name: &str, cell: Arc<Cell>) -> Self {
        let key_hash = tep64_key_hash(name);
        Self {
            key_hash,
            known_key: known_key_by_hash(key_hash),
            value: decode_field_value(cell.clone()),
            raw: cell,
        }
    }

    /// Returns the original field name for known keys.
    pub fn name(&self) -> Option<&'static str> {
        self.known_key.map(Tep64KnownKey::as_str)
    }
}

/// Parses off-chain content stored without the TEP-64 prefix byte.
///
/// Stablecoin jetton masters store the URI as a bare snake string.
pub fn parse_tep64_offchain(cell: Arc<Cell>) -> Result<Tep64Content, MetadataError> {
    let mut slice = Slice::new(cell.clone());
    let uri = flatten_snake_slice(&mut slice)?;
    Ok(Tep64Content::OffChain { uri, raw: cell })
}

fn encode_chunks(bytes: &[u8]) -> Result<Vec<Tep64Chunk>, MetadataError> {
    if bytes.is_empty() {
        return Ok(vec![Tep64Chunk {
            index: 0,
            bytes: Vec::new(),
        }]);
    }
    let mut chunks = Vec::new();
    for (index, chunk) in bytes.chunks(TEP64_CHUNK_BYTES).enumerate() {
        let index = u32::try_from(index).map_err(|_| MetadataError::UnserializableField {
            key: "chunked".to_string(),
            reason: "chunk index exceeds u32".to_string(),
        })?;
        chunks.push(Tep64Chunk {
            index,
            bytes: chunk.to_vec(),
        });
    }
    Ok(chunks)
}

/// Parses a TEP-64 content cell.
pub fn parse_tep64_content(cell: Arc<Cell>) -> Result<Tep64Content, MetadataError> {
    let mut slice = Slice::new(cell.clone());
    if slice.remaining_bits() < 8 {
        return Ok(Tep64Content::Unsupported {
            tag: None,
            raw: cell,
        });
    }

    let tag = slice
        .load_u8()
        .map_err(|error| MetadataError::MalformedSnake(error.to_string()))?;
    match tag {
        TEP64_OFF_CHAIN_TAG => Ok(Tep64Content::OffChain {
            uri: flatten_snake_slice(&mut slice)?,
            raw: cell,
        }),
        TEP64_ON_CHAIN_TAG => {
            let dict = slice
                .load_hashmap_e_with(TEP64_DICT_KEY_BITS, Slice::load_reference)
                .map_err(|error| MetadataError::MalformedOnChainDictionary(error.to_string()))?;
            if !slice.is_empty() {
                return Err(MetadataError::MalformedOnChainDictionary(
                    "trailing data after metadata dictionary".to_string(),
                ));
            }
            Ok(Tep64Content::OnChain {
                fields: decode_onchain_fields(&dict),
                raw: cell,
            })
        }
        tag => Ok(Tep64Content::Unsupported {
            tag: Some(tag),
            raw: cell,
        }),
    }
}

/// Computes the TEP-64 on-chain dictionary key for a metadata field name.
pub fn tep64_key_hash(name: &str) -> [u8; 32] {
    Sha256::digest(name.as_bytes()).into()
}

fn decode_onchain_fields(dict: &HashmapE<Arc<Cell>>) -> Vec<Tep64Field> {
    dict.iter()
        .map(|(key, raw)| {
            let key_hash = bit_key_hash(key);
            Tep64Field {
                key_hash,
                known_key: known_key_by_hash(key_hash),
                value: decode_field_value(raw.clone()),
                raw: raw.clone(),
            }
        })
        .collect()
}

fn decode_field_value(raw: Arc<Cell>) -> Tep64Value {
    let mut slice = Slice::new(raw.clone());
    if slice.remaining_bits() < 8 {
        return match flatten_snake_cell(raw) {
            Ok(bytes) => Tep64Value::Unsupported { tag: None, bytes },
            Err(error) => Tep64Value::Malformed {
                error: error.to_string(),
            },
        };
    }

    let tag = match slice.load_u8() {
        Ok(tag) => tag,
        Err(error) => {
            return Tep64Value::Malformed {
                error: error.to_string(),
            };
        }
    };
    match tag {
        TEP64_ON_CHAIN_TAG => match flatten_snake_slice(&mut slice) {
            Ok(bytes) => Tep64Value::Snake(bytes),
            Err(error) => Tep64Value::Malformed {
                error: error.to_string(),
            },
        },
        TEP64_OFF_CHAIN_TAG => match decode_chunked_slice(&mut slice) {
            Ok((chunks, bytes)) => Tep64Value::Chunked { chunks, bytes },
            Err(error) => Tep64Value::Malformed {
                error: error.to_string(),
            },
        },
        tag => match flatten_snake_cell(raw) {
            Ok(bytes) => Tep64Value::Unsupported {
                tag: Some(tag),
                bytes,
            },
            Err(error) => Tep64Value::Malformed {
                error: error.to_string(),
            },
        },
    }
}

fn decode_chunked_slice(slice: &mut Slice) -> Result<(Vec<Tep64Chunk>, Vec<u8>), MetadataError> {
    let dict = slice
        .load_hashmap_e_with(TEP64_CHUNK_KEY_BITS, Slice::load_reference)
        .map_err(|error| MetadataError::MalformedChunkedDictionary(error.to_string()))?;
    if !slice.is_empty() {
        return Err(MetadataError::MalformedChunkedDictionary(
            "trailing data after chunk dictionary".to_string(),
        ));
    }

    let mut chunks = Vec::with_capacity(dict.len());
    let mut bytes = Vec::new();
    for (key, cell) in dict.iter() {
        let index = u32::try_from(
            key.to_u64()
                .map_err(|error| MetadataError::MalformedChunkedDictionary(error.to_string()))?,
        )
        .map_err(|error| MetadataError::MalformedChunkedDictionary(error.to_string()))?;
        let chunk_bytes = flatten_snake_cell(cell.clone())?
            .ok_or_else(|| MetadataError::MalformedSnake("empty chunk cell".to_string()))?;
        bytes.extend_from_slice(&chunk_bytes);
        chunks.push(Tep64Chunk {
            index,
            bytes: chunk_bytes,
        });
    }
    Ok((chunks, bytes))
}

fn flatten_snake_cell(cell: Arc<Cell>) -> Result<Option<Vec<u8>>, MetadataError> {
    let mut slice = Slice::new(cell);
    if slice.remaining_bits() == 0 && slice.remaining_refs() == 0 {
        return Ok(None);
    }
    flatten_snake_slice(&mut slice).map(Some)
}

fn flatten_snake_slice(slice: &mut Slice) -> Result<Vec<u8>, MetadataError> {
    let mut bytes = Vec::new();
    loop {
        let bits = slice.remaining_bits();
        if !bits.is_multiple_of(8) {
            return Err(MetadataError::NonByteAlignedSnake { bits });
        }
        bytes.extend(
            slice
                .load_bytes(bits / 8)
                .map_err(|error| MetadataError::MalformedSnake(error.to_string()))?,
        );

        match slice.remaining_refs() {
            0 => return Ok(bytes),
            1 => {
                let next = slice
                    .load_reference()
                    .map_err(|error| MetadataError::MalformedSnake(error.to_string()))?;
                *slice = Slice::new(next);
            }
            refs => return Err(MetadataError::TooManySnakeRefs { refs }),
        }
    }
}

fn bit_key_hash(key: &BitKey) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(key.data());
    hash
}

fn known_key_by_hash(hash: [u8; 32]) -> Option<Tep64KnownKey> {
    [
        Tep64KnownKey::Uri,
        Tep64KnownKey::Name,
        Tep64KnownKey::Description,
        Tep64KnownKey::Image,
        Tep64KnownKey::ImageData,
        Tep64KnownKey::Symbol,
        Tep64KnownKey::Decimals,
        Tep64KnownKey::AmountStyle,
        Tep64KnownKey::RenderType,
        Tep64KnownKey::ContentUrl,
        Tep64KnownKey::Video,
    ]
    .into_iter()
    .find(|key| key.key_hash() == hash)
}

#[cfg(test)]
mod tests {
    use crate::tvm::{Builder, deserialize_boc, serialize_boc};

    use super::{
        Arc, BitKey, Cell, HashmapE, MetadataError, TEP64_CHUNK_KEY_BITS, TEP64_DICT_KEY_BITS,
        TEP64_OFF_CHAIN_TAG, TEP64_ON_CHAIN_TAG, Tep64Chunk, Tep64Content, Tep64KnownKey,
        Tep64Value, parse_tep64_content, tep64_key_hash,
    };

    fn snake_cell(bytes: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_snake_bytes(bytes).unwrap();
        builder.build().unwrap()
    }

    fn prefixed_value(tag: u8, bytes: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(tag).unwrap();
        builder.store_snake_bytes(bytes).unwrap();
        builder.build().unwrap()
    }

    fn key(name: &str) -> BitKey {
        BitKey::from_bits(tep64_key_hash(name).to_vec(), TEP64_DICT_KEY_BITS).unwrap()
    }

    #[test]
    fn parses_offchain_uri_snake_content() {
        let mut builder = Builder::new();
        builder.store_u8(TEP64_OFF_CHAIN_TAG).unwrap();
        builder
            .store_snake_bytes(b"https://example.test/meta.json")
            .unwrap();
        let cell = builder.build().unwrap();

        let content = parse_tep64_content(cell.clone()).unwrap();

        assert_eq!(
            content,
            Tep64Content::OffChain {
                uri: b"https://example.test/meta.json".to_vec(),
                raw: cell,
            }
        );
    }

    #[test]
    fn parses_onchain_known_unknown_and_chunked_fields() {
        let mut chunks = HashmapE::new(TEP64_CHUNK_KEY_BITS);
        chunks
            .insert_bit_key(
                BitKey::from_u64(1, TEP64_CHUNK_KEY_BITS).unwrap(),
                snake_cell(b"bar"),
            )
            .unwrap();
        chunks
            .insert_bit_key(
                BitKey::from_u64(0, TEP64_CHUNK_KEY_BITS).unwrap(),
                snake_cell(b"foo"),
            )
            .unwrap();
        let mut chunk_builder = Builder::new();
        chunk_builder.store_u8(TEP64_OFF_CHAIN_TAG).unwrap();
        chunk_builder
            .store_hashmap_e_with(&chunks, |builder, chunk| {
                builder.store_ref(chunk.clone())?;
                Ok(())
            })
            .unwrap();
        let chunked = chunk_builder.build().unwrap();

        let mut fields = HashmapE::new(TEP64_DICT_KEY_BITS);
        fields
            .insert_bit_key(key("name"), prefixed_value(TEP64_ON_CHAIN_TAG, b"Token"))
            .unwrap();
        fields
            .insert_bit_key(key("image_data"), chunked.clone())
            .unwrap();
        fields
            .insert_bit_key(key("custom"), prefixed_value(0xff, b"raw"))
            .unwrap();

        let mut builder = Builder::new();
        builder.store_u8(TEP64_ON_CHAIN_TAG).unwrap();
        builder
            .store_hashmap_e_with(&fields, |builder, value| {
                builder.store_ref(value.clone())?;
                Ok(())
            })
            .unwrap();
        let cell = builder.build().unwrap();

        let Tep64Content::OnChain { fields, raw } = parse_tep64_content(cell.clone()).unwrap()
        else {
            panic!("expected on-chain metadata");
        };
        assert_eq!(raw.hash(), cell.hash());
        assert_eq!(fields.len(), 3);

        let name = fields
            .iter()
            .find(|field| field.known_key == Some(Tep64KnownKey::Name))
            .unwrap();
        assert_eq!(name.value, Tep64Value::Snake(b"Token".to_vec()));

        let image_data = fields
            .iter()
            .find(|field| field.known_key == Some(Tep64KnownKey::ImageData))
            .unwrap();
        assert_eq!(
            image_data.value,
            Tep64Value::Chunked {
                chunks: vec![
                    Tep64Chunk {
                        index: 0,
                        bytes: b"foo".to_vec(),
                    },
                    Tep64Chunk {
                        index: 1,
                        bytes: b"bar".to_vec(),
                    },
                ],
                bytes: b"foobar".to_vec(),
            }
        );

        let custom = fields
            .iter()
            .find(|field| field.known_key.is_none())
            .unwrap();
        assert_eq!(
            custom.value,
            Tep64Value::Unsupported {
                tag: Some(0xff),
                bytes: Some(vec![0xff, b'r', b'a', b'w']),
            }
        );
    }

    #[test]
    fn preserves_unsupported_top_level_content() {
        let mut builder = Builder::new();
        builder.store_u8(0x99).unwrap();
        builder.store_bytes(b"future").unwrap();
        let cell = builder.build().unwrap();

        assert_eq!(
            parse_tep64_content(cell.clone()).unwrap(),
            Tep64Content::Unsupported {
                tag: Some(0x99),
                raw: cell,
            }
        );
    }

    #[test]
    fn rejects_malformed_top_level_snake_but_preserves_field_malformed_value() {
        let mut malformed = Builder::new();
        malformed.store_u8(TEP64_OFF_CHAIN_TAG).unwrap();
        malformed.store_bit(true).unwrap();
        assert!(matches!(
            parse_tep64_content(malformed.build().unwrap()).unwrap_err(),
            MetadataError::NonByteAlignedSnake { bits: 1 }
        ));

        let mut value = Builder::new();
        value.store_u8(TEP64_ON_CHAIN_TAG).unwrap();
        value.store_bit(true).unwrap();
        let mut fields = HashmapE::new(TEP64_DICT_KEY_BITS);
        fields
            .insert_bit_key(key("name"), value.build().unwrap())
            .unwrap();
        let mut root = Builder::new();
        root.store_u8(TEP64_ON_CHAIN_TAG).unwrap();
        root.store_hashmap_e_with(&fields, |builder, value| {
            builder.store_ref(value.clone())?;
            Ok(())
        })
        .unwrap();

        let Tep64Content::OnChain { fields, .. } =
            parse_tep64_content(root.build().unwrap()).unwrap()
        else {
            panic!("expected on-chain metadata");
        };
        assert!(matches!(
            &fields[0].value,
            Tep64Value::Malformed { error } if error.contains("byte-aligned")
        ));
    }

    #[test]
    fn parser_outputs_reparseable_raw_cells() {
        let cell = prefixed_value(TEP64_OFF_CHAIN_TAG, b"https://example.test/meta.json");
        let boc = serialize_boc(&cell, false).unwrap();
        let reparsed = deserialize_boc(&boc).unwrap();
        assert_eq!(
            parse_tep64_content(reparsed).unwrap().raw().hash(),
            cell.hash()
        );
    }

    trait RawContent {
        fn raw(&self) -> &Arc<Cell>;
    }

    impl RawContent for Tep64Content {
        fn raw(&self) -> &Arc<Cell> {
            match self {
                Tep64Content::OffChain { raw, .. }
                | Tep64Content::OnChain { raw, .. }
                | Tep64Content::Unsupported { raw, .. } => raw,
            }
        }
    }
}
