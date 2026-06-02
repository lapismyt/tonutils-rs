use super::*;

use std::sync::Arc;

use num_bigint::{BigInt, BigUint, Sign};
use thiserror::Error;

use crate::tlb::{MsgAddress, MsgAddressInt, TlbDeserialize, TlbSerialize};
use crate::tvm::{Address, Builder, Cell, Slice, TvmStackEntry};

/// Maximum integer width accepted by the ABI model.
pub const ABI_INTEGER_MAX_BITS: u16 = 257;

/// A complete ABI document grouped by contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiDefinition {
    /// Human-readable ABI name.
    pub name: String,
    /// ABI format or producer version string.
    pub version: String,
    /// Contracts described by this ABI document.
    pub contracts: Vec<AbiContract>,
}

impl AbiDefinition {
    /// Validates local data-model invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        ensure_non_empty("ABI definition name", &self.name)?;
        ensure_non_empty("ABI definition version", &self.version)?;
        for contract in &self.contracts {
            contract.validate()?;
        }
        Ok(())
    }
}

/// ABI surface for one contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiContract {
    /// Contract name.
    pub name: String,
    /// Callable methods or message handlers.
    pub methods: Vec<AbiFunction>,
    /// Events emitted by the contract.
    pub events: Vec<AbiEvent>,
}

impl AbiContract {
    /// Validates local data-model invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        ensure_non_empty("contract name", &self.name)?;
        for method in &self.methods {
            method.validate()?;
        }
        for event in &self.events {
            event.validate()?;
        }
        Ok(())
    }
}

/// ABI description of a contract function or message handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiFunction {
    /// Function or handler name.
    pub name: String,
    /// Execution surface described by this function.
    pub kind: AbiFunctionKind,
    /// Optional method id or message opcode.
    pub selector: AbiSelector,
    /// Input parameters.
    pub inputs: Vec<AbiParameter>,
    /// Output parameters.
    pub outputs: Vec<AbiParameter>,
}

impl AbiFunction {
    /// Validates local data-model invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        ensure_non_empty("function name", &self.name)?;
        for input in &self.inputs {
            input.validate()?;
        }
        for output in &self.outputs {
            output.validate()?;
        }
        Ok(())
    }
}

/// ABI description of an emitted event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiEvent {
    /// Event name.
    pub name: String,
    /// Optional event selector or opcode.
    pub selector: AbiSelector,
    /// Event fields.
    pub fields: Vec<AbiParameter>,
}

impl AbiEvent {
    /// Validates local data-model invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        ensure_non_empty("event name", &self.name)?;
        for field in &self.fields {
            field.validate()?;
        }
        Ok(())
    }
}

/// Named ABI parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiParameter {
    /// Parameter or field name.
    pub name: String,
    /// Parameter type.
    pub ty: AbiType,
    /// Whether the parameter may be absent at the ABI layer.
    pub optional: bool,
}

impl AbiParameter {
    /// Validates local data-model invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        ensure_non_empty("parameter name", &self.name)?;
        self.ty.validate()
    }
}

/// ABI type vocabulary for TON and TVM-oriented contract data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiType {
    /// Signed integer with an explicit bit width.
    Int { bits: u16 },
    /// Unsigned integer with an explicit bit width.
    Uint { bits: u16 },
    /// TVM boolean value.
    Bool,
    /// Byte string.
    Bytes,
    /// UTF-8 string.
    String,
    /// TON address value.
    Address,
    /// TVM cell value.
    Cell,
    /// TVM slice value.
    Slice,
    /// Named tuple fields.
    Tuple(Vec<AbiParameter>),
    /// Homogeneous array.
    Array(Box<AbiType>),
    /// Dictionary-like key/value mapping.
    Map {
        /// Key type.
        key: Box<AbiType>,
        /// Value type.
        value: Box<AbiType>,
        /// Explicit dictionary key width. Integer key widths are used when
        /// this is omitted.
        key_bits: Option<u16>,
    },
    /// Optional nested value.
    Optional(Box<AbiType>),
    /// Raw type spelling preserved for future parser and compatibility work.
    Unknown(String),
}

impl AbiType {
    /// Validates recursive type invariants.
    pub fn validate(&self) -> Result<(), AbiModelError> {
        match self {
            Self::Int { bits } => validate_integer_width("int", *bits),
            Self::Uint { bits } => validate_integer_width("uint", *bits),
            Self::Bool | Self::Bytes | Self::String | Self::Address | Self::Cell | Self::Slice => {
                Ok(())
            }
            Self::Tuple(fields) => {
                for field in fields {
                    field.validate()?;
                }
                Ok(())
            }
            Self::Array(item) | Self::Optional(item) => item.validate(),
            Self::Map { key, value, .. } => {
                key.validate()?;
                value.validate()
            }
            Self::Unknown(name) => ensure_non_empty("unknown type name", name),
        }
    }
}

/// Runtime value paired with an [`AbiType`] for stack conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiValue {
    /// Signed integer value.
    Int(BigInt),
    /// Unsigned integer value.
    Uint(BigUint),
    /// TVM boolean value.
    Bool(bool),
    /// Byte string value.
    Bytes(Vec<u8>),
    /// UTF-8 string value.
    String(String),
    /// Standard TON internal address.
    Address(Address),
    /// Owned TVM cell value.
    Cell(Arc<Cell>),
    /// Owned TVM slice value, represented by its backing cell.
    Slice(Arc<Cell>),
    /// Tuple values in ABI field order.
    Tuple(Vec<AbiValue>),
    /// Homogeneous array values.
    Array(Vec<AbiValue>),
    /// Dictionary entries in ABI key/value form.
    Map(Vec<(AbiValue, AbiValue)>),
    /// Optional nested value.
    Optional(Option<Box<AbiValue>>),
}

impl AbiValue {
    /// Converts this ABI value into a TVM stack entry according to `ty`.
    pub fn to_stack_entry(&self, ty: &AbiType) -> Result<TvmStackEntry, AbiCodecError> {
        abi_value_to_stack_entry(self, ty)
    }

    /// Decodes a TVM stack entry into an ABI value according to `ty`.
    pub fn from_stack_entry(ty: &AbiType, entry: &TvmStackEntry) -> Result<Self, AbiCodecError> {
        abi_value_from_stack_entry(ty, entry)
    }
}

/// Converts an ABI value into a TVM stack entry according to `ty`.
pub fn to_stack_entry(value: &AbiValue, ty: &AbiType) -> Result<TvmStackEntry, AbiCodecError> {
    abi_value_to_stack_entry(value, ty)
}

/// Converts an ABI value into a TVM stack entry according to `ty`.
pub fn abi_value_to_stack_entry(
    value: &AbiValue,
    ty: &AbiType,
) -> Result<TvmStackEntry, AbiCodecError> {
    match (ty, value) {
        (AbiType::Int { bits }, AbiValue::Int(value)) => {
            validate_integer_width_codec("int", *bits)?;
            ensure_signed_range(value, *bits)?;
            Ok(TvmStackEntry::Int(value.clone()))
        }
        (AbiType::Uint { bits }, AbiValue::Uint(value)) => {
            validate_integer_width_codec("uint", *bits)?;
            ensure_unsigned_range(value, *bits)?;
            Ok(TvmStackEntry::Int(BigInt::from(value.clone())))
        }
        (AbiType::Bool, AbiValue::Bool(value)) => Ok(TvmStackEntry::Int(if *value {
            BigInt::from(-1)
        } else {
            BigInt::from(0)
        })),
        (AbiType::Bytes, AbiValue::Bytes(bytes)) => Ok(TvmStackEntry::Cell(snake_cell(bytes)?)),
        (AbiType::String, AbiValue::String(value)) => {
            Ok(TvmStackEntry::Cell(snake_cell(value.as_bytes())?))
        }
        (AbiType::Address, AbiValue::Address(address)) => {
            Ok(TvmStackEntry::Slice(address_slice_cell(address)?))
        }
        (AbiType::Cell, AbiValue::Cell(cell)) => Ok(TvmStackEntry::Cell(cell.clone())),
        (AbiType::Slice, AbiValue::Slice(cell)) => Ok(TvmStackEntry::Slice(cell.clone())),
        (AbiType::Tuple(fields), AbiValue::Tuple(values)) => {
            if fields.len() != values.len() {
                return Err(AbiCodecError::ArityMismatch {
                    kind: "tuple",
                    expected: fields.len(),
                    actual: values.len(),
                });
            }
            let entries = fields
                .iter()
                .zip(values)
                .map(|(field, value)| abi_value_to_stack_entry(value, &field.ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TvmStackEntry::Tuple(entries))
        }
        (AbiType::Array(item_ty), AbiValue::Array(values)) => {
            let entries = values
                .iter()
                .map(|value| abi_value_to_stack_entry(value, item_ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TvmStackEntry::List(entries))
        }
        (AbiType::Optional(_), AbiValue::Optional(None)) => Ok(TvmStackEntry::Null),
        (AbiType::Optional(item_ty), AbiValue::Optional(Some(value))) => {
            abi_value_to_stack_entry(value, item_ty)
        }
        (
            AbiType::Map {
                key,
                value,
                key_bits,
            },
            AbiValue::Map(entries),
        ) => Ok(TvmStackEntry::Cell(encode_map_cell(
            key, value, *key_bits, entries,
        )?)),
        (AbiType::Unknown(_), _) => Err(AbiCodecError::UnsupportedType { ty: "unknown" }),
        (ty, value) => Err(AbiCodecError::TypeMismatch {
            expected: abi_type_name(ty),
            actual: abi_value_name(value),
        }),
    }
}

/// Decodes a TVM stack entry into an ABI value according to `ty`.
pub fn from_stack_entry(ty: &AbiType, entry: &TvmStackEntry) -> Result<AbiValue, AbiCodecError> {
    abi_value_from_stack_entry(ty, entry)
}

/// Decodes a TVM stack entry into an ABI value according to `ty`.
pub fn abi_value_from_stack_entry(
    ty: &AbiType,
    entry: &TvmStackEntry,
) -> Result<AbiValue, AbiCodecError> {
    match ty {
        AbiType::Int { bits } => {
            validate_integer_width_codec("int", *bits)?;
            let value = stack_int(entry, "int")?;
            ensure_signed_range(value, *bits)?;
            Ok(AbiValue::Int(value.clone()))
        }
        AbiType::Uint { bits } => {
            validate_integer_width_codec("uint", *bits)?;
            let value = stack_int(entry, "uint")?;
            if value.sign() == Sign::Minus {
                return Err(AbiCodecError::IntegerOutOfRange {
                    kind: "uint",
                    bits: *bits,
                    value: value.to_string(),
                });
            }
            let value = value
                .to_biguint()
                .ok_or_else(|| AbiCodecError::IntegerOutOfRange {
                    kind: "uint",
                    bits: *bits,
                    value: value.to_string(),
                })?;
            ensure_unsigned_range(&value, *bits)?;
            Ok(AbiValue::Uint(value))
        }
        AbiType::Bool => {
            let value = stack_int(entry, "bool")?;
            if *value == BigInt::from(-1) {
                Ok(AbiValue::Bool(true))
            } else if *value == BigInt::from(0) {
                Ok(AbiValue::Bool(false))
            } else {
                Err(AbiCodecError::InvalidBool {
                    value: value.to_string(),
                })
            }
        }
        AbiType::Bytes => {
            let cell = stack_cell(entry, "bytes")?;
            Ok(AbiValue::Bytes(read_snake_cell(cell.clone())?))
        }
        AbiType::String => {
            let cell = stack_cell(entry, "string")?;
            let bytes = read_snake_cell(cell.clone())?;
            let value = String::from_utf8(bytes).map_err(|source| AbiCodecError::InvalidUtf8 {
                error: source.to_string(),
            })?;
            Ok(AbiValue::String(value))
        }
        AbiType::Address => {
            let cell = stack_slice(entry, "address")?;
            decode_address_slice(cell.clone()).map(AbiValue::Address)
        }
        AbiType::Cell => match entry {
            TvmStackEntry::Cell(cell) => Ok(AbiValue::Cell(cell.clone())),
            other => Err(AbiCodecError::TypeMismatch {
                expected: "cell",
                actual: stack_entry_name(other),
            }),
        },
        AbiType::Slice => match entry {
            TvmStackEntry::Slice(cell) => Ok(AbiValue::Slice(cell.clone())),
            other => Err(AbiCodecError::TypeMismatch {
                expected: "slice",
                actual: stack_entry_name(other),
            }),
        },
        AbiType::Tuple(fields) => match entry {
            TvmStackEntry::Tuple(entries) => {
                if fields.len() != entries.len() {
                    return Err(AbiCodecError::ArityMismatch {
                        kind: "tuple",
                        expected: fields.len(),
                        actual: entries.len(),
                    });
                }
                let values = fields
                    .iter()
                    .zip(entries)
                    .map(|(field, entry)| abi_value_from_stack_entry(&field.ty, entry))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(AbiValue::Tuple(values))
            }
            other => Err(AbiCodecError::TypeMismatch {
                expected: "tuple",
                actual: stack_entry_name(other),
            }),
        },
        AbiType::Array(item_ty) => match entry {
            TvmStackEntry::List(entries) => {
                let values = entries
                    .iter()
                    .map(|entry| abi_value_from_stack_entry(item_ty, entry))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(AbiValue::Array(values))
            }
            other => Err(AbiCodecError::TypeMismatch {
                expected: "list",
                actual: stack_entry_name(other),
            }),
        },
        AbiType::Optional(item_ty) => match entry {
            TvmStackEntry::Null => Ok(AbiValue::Optional(None)),
            entry => Ok(AbiValue::Optional(Some(Box::new(
                abi_value_from_stack_entry(item_ty, entry)?,
            )))),
        },
        AbiType::Map {
            key,
            value,
            key_bits,
        } => {
            let cell = stack_cell(entry, "map")?;
            decode_map_cell(key, value, *key_bits, cell.clone()).map(AbiValue::Map)
        }
        AbiType::Unknown(_) => Err(AbiCodecError::UnsupportedType { ty: "unknown" }),
    }
}

/// Error returned when ABI values cannot be converted to or from TVM stack data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AbiCodecError {
    /// The ABI type, runtime value, or stack entry kind does not match.
    #[error("ABI codec type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        /// Expected type name.
        expected: &'static str,
        /// Actual value or stack entry type name.
        actual: &'static str,
    },
    /// Tuple or array arity does not match the ABI declaration.
    #[error("ABI codec {kind} arity mismatch: expected {expected}, got {actual}")]
    ArityMismatch {
        /// Aggregate kind.
        kind: &'static str,
        /// Expected number of items.
        expected: usize,
        /// Actual number of items.
        actual: usize,
    },
    /// An integer type uses an unsupported bit width.
    #[error("{kind} bit width must be in 1..={max}, got {bits}")]
    InvalidIntegerWidth {
        /// Integer family.
        kind: &'static str,
        /// Rejected bit width.
        bits: u16,
        /// Maximum accepted bit width.
        max: u16,
    },
    /// A runtime integer value does not fit the declared ABI width.
    #[error("{kind}{bits} value {value} is outside the declared range")]
    IntegerOutOfRange {
        /// Integer family.
        kind: &'static str,
        /// Declared bit width.
        bits: u16,
        /// Rejected value in decimal.
        value: String,
    },
    /// A stack integer is not a canonical TVM boolean.
    #[error("TVM bool stack value must be -1 or 0, got {value}")]
    InvalidBool {
        /// Rejected integer value in decimal.
        value: String,
    },
    /// String decoding failed because bytes are not valid UTF-8.
    #[error("ABI string stack value is not valid UTF-8: {error}")]
    InvalidUtf8 {
        /// UTF-8 decoder error.
        error: String,
    },
    /// Snake-cell byte payload is malformed.
    #[error("ABI snake cell payload is malformed: {reason}")]
    MalformedSnake {
        /// Failure reason.
        reason: String,
    },
    /// Address stack slice payload is malformed or unsupported.
    #[error("ABI address stack value is malformed: {reason}")]
    MalformedAddress {
        /// Failure reason.
        reason: String,
    },
    /// Conversion for this ABI type is intentionally not defined yet.
    #[error("ABI conversion for {ty} is unsupported")]
    UnsupportedType {
        /// ABI type family.
        ty: &'static str,
    },
    /// ABI map keys are limited to fixed-width integer types.
    #[error("ABI map key type {ty} is unsupported")]
    UnsupportedMapKey {
        /// Rejected key type.
        ty: &'static str,
    },
    /// ABI map key bit width is not compatible with the key type.
    #[error("ABI map key width must be {expected} bits for {kind}{expected}, got {actual}")]
    InvalidMapKeyBits {
        /// Integer family.
        kind: &'static str,
        /// Width required by the ABI key type.
        expected: u16,
        /// Explicit key width.
        actual: u16,
    },
    /// Two ABI map entries encode to the same dictionary key.
    #[error("ABI map contains duplicate encoded key {key_hex}")]
    DuplicateMapKey {
        /// Canonical encoded key bytes, hex encoded.
        key_hex: String,
    },
    /// Message body construction was requested for a non-message function or
    /// with a selector that is not valid for message bodies.
    #[error("ABI message body selector {selector:?} is invalid for {kind:?}")]
    InvalidMessageSelector {
        /// Function kind being encoded or decoded.
        kind: AbiFunctionKind,
        /// Selector attached to the function.
        selector: AbiSelector,
    },
    /// Get-method stack conversion was requested for a non-get function or
    /// with a selector that is not valid for get-method calls.
    #[error("ABI get-method selector {selector:?} is invalid for {kind:?}")]
    InvalidGetMethodSelector {
        /// Function kind being encoded or decoded.
        kind: AbiFunctionKind,
        /// Selector attached to the function.
        selector: AbiSelector,
    },
    /// Event payload construction was requested with a selector kind that is
    /// not valid for events.
    #[error("ABI event selector {selector:?} is invalid")]
    InvalidEventSelector {
        /// Selector attached to the event.
        selector: AbiSelector,
    },
    /// The body opcode does not match the ABI selector.
    #[error("ABI message body opcode mismatch: expected {expected:#010x}, got {actual:#010x}")]
    OpcodeMismatch {
        /// Expected opcode.
        expected: u32,
        /// Actual opcode read from the body.
        actual: u32,
    },
    /// Message body cell data is malformed for the declared ABI layout.
    #[error("ABI message body is malformed: {reason}")]
    MalformedBody {
        /// Failure reason.
        reason: String,
    },
    /// Exact message body decode left unread data.
    #[error("ABI message body has trailing data: {bits} bits and {refs} refs remaining")]
    TrailingBodyData {
        /// Unread bit count.
        bits: usize,
        /// Unread reference count.
        refs: usize,
    },
}

/// Contract entry point class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiFunctionKind {
    /// Read-only get-method execution.
    GetMethod,
    /// Internal inbound message handler.
    InternalMessage,
    /// External inbound message handler.
    ExternalMessage,
}

/// Numeric selector associated with a function or event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiSelector {
    /// No selector is known or required.
    None,
    /// TVM get-method id.
    MethodId(u64),
    /// Message or event opcode.
    Opcode(u32),
}

/// Error returned when an ABI model violates local structural invariants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AbiModelError {
    /// A required name field is empty or whitespace only.
    #[error("{field} must not be empty")]
    EmptyName {
        /// Name field that failed validation.
        field: &'static str,
    },
    /// An integer type uses an unsupported bit width.
    #[error("{kind} bit width must be in 1..={max}, got {bits}")]
    InvalidIntegerWidth {
        /// Integer family.
        kind: &'static str,
        /// Rejected bit width.
        bits: u16,
        /// Maximum accepted bit width.
        max: u16,
    },
}

pub(super) fn ensure_non_empty(field: &'static str, value: &str) -> Result<(), AbiModelError> {
    if value.trim().is_empty() {
        return Err(AbiModelError::EmptyName { field });
    }
    Ok(())
}

pub(super) fn validate_integer_width(kind: &'static str, bits: u16) -> Result<(), AbiModelError> {
    if !(1..=ABI_INTEGER_MAX_BITS).contains(&bits) {
        return Err(AbiModelError::InvalidIntegerWidth {
            kind,
            bits,
            max: ABI_INTEGER_MAX_BITS,
        });
    }
    Ok(())
}

pub(super) fn validate_integer_width_codec(
    kind: &'static str,
    bits: u16,
) -> Result<(), AbiCodecError> {
    if !(1..=ABI_INTEGER_MAX_BITS).contains(&bits) {
        return Err(AbiCodecError::InvalidIntegerWidth {
            kind,
            bits,
            max: ABI_INTEGER_MAX_BITS,
        });
    }
    Ok(())
}

pub(super) fn ensure_signed_range(value: &BigInt, bits: u16) -> Result<(), AbiCodecError> {
    let min = -(BigInt::from(1) << (bits - 1));
    let max = (BigInt::from(1) << (bits - 1)) - 1;
    if value < &min || value > &max {
        return Err(AbiCodecError::IntegerOutOfRange {
            kind: "int",
            bits,
            value: value.to_string(),
        });
    }
    Ok(())
}

pub(super) fn ensure_unsigned_range(value: &BigUint, bits: u16) -> Result<(), AbiCodecError> {
    let max_exclusive = BigUint::from(1u8) << bits;
    if value >= &max_exclusive {
        return Err(AbiCodecError::IntegerOutOfRange {
            kind: "uint",
            bits,
            value: value.to_string(),
        });
    }
    Ok(())
}

pub(super) fn snake_cell(bytes: &[u8]) -> Result<Arc<Cell>, AbiCodecError> {
    let mut builder = Builder::new();
    builder
        .store_snake_bytes(bytes)
        .map_err(|source| AbiCodecError::MalformedSnake {
            reason: source.to_string(),
        })?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedSnake {
            reason: source.to_string(),
        })
}

pub(super) fn read_snake_cell(cell: Arc<Cell>) -> Result<Vec<u8>, AbiCodecError> {
    let mut bytes = Vec::new();
    read_snake_cell_into(cell, &mut bytes)?;
    Ok(bytes)
}

pub(super) fn read_snake_cell_into(
    cell: Arc<Cell>,
    bytes: &mut Vec<u8>,
) -> Result<(), AbiCodecError> {
    let mut slice = Slice::new(cell);
    if !slice.remaining_bits().is_multiple_of(8) {
        return Err(AbiCodecError::MalformedSnake {
            reason: format!(
                "cell has {} trailing bits, expected byte-aligned data",
                slice.remaining_bits()
            ),
        });
    }
    let byte_len = slice.remaining_bits() / 8;
    bytes.extend(
        slice
            .load_bytes(byte_len)
            .map_err(|source| AbiCodecError::MalformedSnake {
                reason: source.to_string(),
            })?,
    );

    match slice.remaining_refs() {
        0 => Ok(()),
        1 => {
            let next = slice
                .load_reference()
                .map_err(|source| AbiCodecError::MalformedSnake {
                    reason: source.to_string(),
                })?;
            read_snake_cell_into(next, bytes)
        }
        refs => Err(AbiCodecError::MalformedSnake {
            reason: format!("cell has {refs} continuation references, expected at most 1"),
        }),
    }
}

pub(super) fn address_slice_cell(address: &Address) -> Result<Arc<Cell>, AbiCodecError> {
    MsgAddressInt::std(address.clone())
        .to_cell()
        .map_err(|source| AbiCodecError::MalformedAddress {
            reason: source.to_string(),
        })
}

pub(super) fn decode_address_slice(cell: Arc<Cell>) -> Result<Address, AbiCodecError> {
    let mut slice = Slice::new(cell);
    let address =
        MsgAddress::load_tlb(&mut slice).map_err(|source| AbiCodecError::MalformedAddress {
            reason: source.to_string(),
        })?;
    if !slice.is_empty() {
        return Err(AbiCodecError::MalformedAddress {
            reason: "address slice has trailing data".to_string(),
        });
    }

    match address {
        MsgAddress::Int(MsgAddressInt::Std {
            anycast: None,
            address,
        }) => Ok(address),
        MsgAddress::Int(MsgAddressInt::Std {
            anycast: Some(_), ..
        }) => Err(AbiCodecError::MalformedAddress {
            reason: "standard address contains anycast".to_string(),
        }),
        MsgAddress::Int(MsgAddressInt::Var { .. }) => Err(AbiCodecError::MalformedAddress {
            reason: "variable-length internal addresses are unsupported".to_string(),
        }),
        MsgAddress::Ext(_) => Err(AbiCodecError::MalformedAddress {
            reason: "external addresses are unsupported".to_string(),
        }),
    }
}

pub(super) fn stack_int<'a>(
    entry: &'a TvmStackEntry,
    expected: &'static str,
) -> Result<&'a BigInt, AbiCodecError> {
    match entry {
        TvmStackEntry::Int(value) => Ok(value),
        other => Err(AbiCodecError::TypeMismatch {
            expected,
            actual: stack_entry_name(other),
        }),
    }
}

pub(super) fn stack_cell<'a>(
    entry: &'a TvmStackEntry,
    expected: &'static str,
) -> Result<&'a Arc<Cell>, AbiCodecError> {
    match entry {
        TvmStackEntry::Cell(cell) => Ok(cell),
        other => Err(AbiCodecError::TypeMismatch {
            expected,
            actual: stack_entry_name(other),
        }),
    }
}

pub(super) fn stack_slice<'a>(
    entry: &'a TvmStackEntry,
    expected: &'static str,
) -> Result<&'a Arc<Cell>, AbiCodecError> {
    match entry {
        TvmStackEntry::Slice(cell) => Ok(cell),
        other => Err(AbiCodecError::TypeMismatch {
            expected,
            actual: stack_entry_name(other),
        }),
    }
}

pub(super) fn abi_type_name(ty: &AbiType) -> &'static str {
    match ty {
        AbiType::Int { .. } => "int",
        AbiType::Uint { .. } => "uint",
        AbiType::Bool => "bool",
        AbiType::Bytes => "bytes",
        AbiType::String => "string",
        AbiType::Address => "address",
        AbiType::Cell => "cell",
        AbiType::Slice => "slice",
        AbiType::Tuple(_) => "tuple",
        AbiType::Array(_) => "array",
        AbiType::Map { .. } => "map",
        AbiType::Optional(_) => "optional",
        AbiType::Unknown(_) => "unknown",
    }
}

pub(super) fn abi_value_name(value: &AbiValue) -> &'static str {
    match value {
        AbiValue::Int(_) => "int",
        AbiValue::Uint(_) => "uint",
        AbiValue::Bool(_) => "bool",
        AbiValue::Bytes(_) => "bytes",
        AbiValue::String(_) => "string",
        AbiValue::Address(_) => "address",
        AbiValue::Cell(_) => "cell",
        AbiValue::Slice(_) => "slice",
        AbiValue::Tuple(_) => "tuple",
        AbiValue::Array(_) => "array",
        AbiValue::Map(_) => "map",
        AbiValue::Optional(_) => "optional",
    }
}

pub(super) fn stack_entry_name(entry: &TvmStackEntry) -> &'static str {
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
