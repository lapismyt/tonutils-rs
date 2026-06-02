use super::*;

use std::sync::Arc;

use crate::tlb::{MsgAddressInt, TlbDeserialize, TlbSerialize};
use crate::tvm::{Builder, Cell, Slice, TvmStackEntry};

/// Encodes ABI get-method input values into TVM stack entries.
pub fn encode_get_method_inputs(
    function: &AbiFunction,
    values: &[AbiValue],
) -> Result<Vec<TvmStackEntry>, AbiCodecError> {
    ensure_get_method_function(function)?;
    if function.inputs.len() != values.len() {
        return Err(AbiCodecError::ArityMismatch {
            kind: "get-method inputs",
            expected: function.inputs.len(),
            actual: values.len(),
        });
    }

    function
        .inputs
        .iter()
        .zip(values)
        .map(|(parameter, value)| abi_value_to_stack_entry(value, &parameter.ty))
        .collect()
}

/// Decodes ABI get-method output values from TVM stack entries.
pub fn decode_get_method_outputs(
    function: &AbiFunction,
    entries: &[TvmStackEntry],
) -> Result<Vec<AbiValue>, AbiCodecError> {
    ensure_get_method_function(function)?;
    if function.outputs.len() != entries.len() {
        return Err(AbiCodecError::ArityMismatch {
            kind: "get-method outputs",
            expected: function.outputs.len(),
            actual: entries.len(),
        });
    }

    function
        .outputs
        .iter()
        .zip(entries)
        .map(|(parameter, entry)| abi_value_from_stack_entry(&parameter.ty, entry))
        .collect()
}

/// Encodes an internal or external message body from ABI input values.
///
/// The current body policy is intentionally conservative: optional message
/// opcodes are encoded as a 32-bit prefix, fixed-width scalar values are
/// encoded inline, addresses are encoded as `MsgAddressInt`, and dynamically
/// sized or cell-like values are stored by reference.
pub fn encode_message_body(
    function: &AbiFunction,
    values: &[AbiValue],
) -> Result<Arc<Cell>, AbiCodecError> {
    ensure_message_function(function)?;

    let mut builder = Builder::new();
    store_message_selector(&mut builder, function)?;
    store_payload_components(&mut builder, &function.inputs, values, "message inputs")?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}

/// Decodes ABI input values from an internal or external message body.
pub fn decode_message_body(
    function: &AbiFunction,
    body: Arc<Cell>,
) -> Result<Vec<AbiValue>, AbiCodecError> {
    ensure_message_function(function)?;

    let mut slice = Slice::new(body);
    load_message_selector(&mut slice, function)?;
    load_payload_components_exact(&mut slice, &function.inputs)
}

/// Encodes ABI values as payload components without a selector prefix.
///
/// This uses the same local payload policy as message bodies for scalar,
/// address, tuple, optional, reference, and map values.
pub fn encode_payload_components(
    parameters: &[AbiParameter],
    values: &[AbiValue],
) -> Result<Arc<Cell>, AbiCodecError> {
    let mut builder = Builder::new();
    store_payload_components(&mut builder, parameters, values, "payload components")?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}

/// Decodes ABI values from payload components without a selector prefix.
///
/// Decoding is exact and rejects trailing bits or references after the
/// declared components are loaded.
pub fn decode_payload_components(
    parameters: &[AbiParameter],
    payload: Arc<Cell>,
) -> Result<Vec<AbiValue>, AbiCodecError> {
    let mut slice = Slice::new(payload);
    load_payload_components_exact(&mut slice, parameters)
}

/// Encodes an event payload using the same local component policy as message
/// bodies.
///
/// `Opcode(u32)` event selectors are encoded as a 32-bit prefix. `None`
/// selectors have no prefix. `MethodId` selectors are rejected because they are
/// only valid for get-method calls.
pub fn encode_event_payload(
    event: &AbiEvent,
    values: &[AbiValue],
) -> Result<Arc<Cell>, AbiCodecError> {
    ensure_event_selector(event.selector)?;

    let mut builder = Builder::new();
    store_selector(&mut builder, event.selector)?;
    store_payload_components(&mut builder, &event.fields, values, "event fields")?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}

/// Decodes an event payload using the same local component policy as message
/// bodies.
pub fn decode_event_payload(
    event: &AbiEvent,
    payload: Arc<Cell>,
) -> Result<Vec<AbiValue>, AbiCodecError> {
    ensure_event_selector(event.selector)?;

    let mut slice = Slice::new(payload);
    load_selector(&mut slice, event.selector)?;
    load_payload_components_exact(&mut slice, &event.fields)
}

fn ensure_message_function(function: &AbiFunction) -> Result<(), AbiCodecError> {
    match (function.kind, function.selector) {
        (
            AbiFunctionKind::InternalMessage | AbiFunctionKind::ExternalMessage,
            AbiSelector::None,
        )
        | (
            AbiFunctionKind::InternalMessage | AbiFunctionKind::ExternalMessage,
            AbiSelector::Opcode(_),
        ) => Ok(()),
        _ => Err(AbiCodecError::InvalidMessageSelector {
            kind: function.kind,
            selector: function.selector,
        }),
    }
}

fn ensure_get_method_function(function: &AbiFunction) -> Result<(), AbiCodecError> {
    match (function.kind, function.selector) {
        (AbiFunctionKind::GetMethod, AbiSelector::None)
        | (AbiFunctionKind::GetMethod, AbiSelector::MethodId(_)) => Ok(()),
        _ => Err(AbiCodecError::InvalidGetMethodSelector {
            kind: function.kind,
            selector: function.selector,
        }),
    }
}

fn store_message_selector(
    builder: &mut Builder,
    function: &AbiFunction,
) -> Result<(), AbiCodecError> {
    store_selector(builder, function.selector)
}

fn load_message_selector(slice: &mut Slice, function: &AbiFunction) -> Result<(), AbiCodecError> {
    load_selector(slice, function.selector)
}

fn ensure_event_selector(selector: AbiSelector) -> Result<(), AbiCodecError> {
    match selector {
        AbiSelector::None | AbiSelector::Opcode(_) => Ok(()),
        AbiSelector::MethodId(_) => Err(AbiCodecError::InvalidEventSelector { selector }),
    }
}

fn store_selector(builder: &mut Builder, selector: AbiSelector) -> Result<(), AbiCodecError> {
    if let AbiSelector::Opcode(opcode) = selector {
        builder
            .store_u32(opcode)
            .map_err(|source| AbiCodecError::MalformedBody {
                reason: source.to_string(),
            })?;
    }
    Ok(())
}

fn load_selector(slice: &mut Slice, selector: AbiSelector) -> Result<(), AbiCodecError> {
    if let AbiSelector::Opcode(expected) = selector {
        let actual = slice
            .load_u32()
            .map_err(|source| AbiCodecError::MalformedBody {
                reason: source.to_string(),
            })?;
        if actual != expected {
            return Err(AbiCodecError::OpcodeMismatch { expected, actual });
        }
    }
    Ok(())
}

fn store_payload_components(
    builder: &mut Builder,
    parameters: &[AbiParameter],
    values: &[AbiValue],
    kind: &'static str,
) -> Result<(), AbiCodecError> {
    if parameters.len() != values.len() {
        return Err(AbiCodecError::ArityMismatch {
            kind,
            expected: parameters.len(),
            actual: values.len(),
        });
    }

    for (parameter, value) in parameters.iter().zip(values) {
        store_body_value(builder, &parameter.ty, value)?;
    }
    Ok(())
}

fn load_payload_components_exact(
    slice: &mut Slice,
    parameters: &[AbiParameter],
) -> Result<Vec<AbiValue>, AbiCodecError> {
    let values = parameters
        .iter()
        .map(|parameter| load_body_value(slice, &parameter.ty))
        .collect::<Result<Vec<_>, _>>()?;

    if !slice.is_empty() {
        return Err(AbiCodecError::TrailingBodyData {
            bits: slice.remaining_bits(),
            refs: slice.remaining_refs(),
        });
    }

    Ok(values)
}

pub(super) fn store_body_value(
    builder: &mut Builder,
    ty: &AbiType,
    value: &AbiValue,
) -> Result<(), AbiCodecError> {
    match (ty, value) {
        (AbiType::Int { bits }, AbiValue::Int(value)) => {
            validate_integer_width_codec("int", *bits)?;
            ensure_signed_range(value, *bits)?;
            builder
                .store_big_int(value, usize::from(*bits))
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            Ok(())
        }
        (AbiType::Uint { bits }, AbiValue::Uint(value)) => {
            validate_integer_width_codec("uint", *bits)?;
            ensure_unsigned_range(value, *bits)?;
            builder
                .store_big_uint(value, usize::from(*bits))
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            Ok(())
        }
        (AbiType::Bool, AbiValue::Bool(value)) => {
            builder
                .store_bool(*value)
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            Ok(())
        }
        (AbiType::Bytes, AbiValue::Bytes(bytes)) => store_body_ref(builder, snake_cell(bytes)?),
        (AbiType::String, AbiValue::String(value)) => {
            store_body_ref(builder, snake_cell(value.as_bytes())?)
        }
        (AbiType::Address, AbiValue::Address(address)) => MsgAddressInt::std(address.clone())
            .store_tlb(builder)
            .map_err(|source| AbiCodecError::MalformedBody {
                reason: source.to_string(),
            }),
        (AbiType::Cell, AbiValue::Cell(cell)) | (AbiType::Slice, AbiValue::Slice(cell)) => {
            store_body_ref(builder, cell.clone())
        }
        (AbiType::Tuple(fields), AbiValue::Tuple(values)) => {
            if fields.len() != values.len() {
                return Err(AbiCodecError::ArityMismatch {
                    kind: "tuple",
                    expected: fields.len(),
                    actual: values.len(),
                });
            }
            for (field, value) in fields.iter().zip(values) {
                store_body_value(builder, &field.ty, value)?;
            }
            Ok(())
        }
        (AbiType::Optional(_), AbiValue::Optional(None)) => {
            builder
                .store_bit(false)
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            Ok(())
        }
        (AbiType::Optional(item_ty), AbiValue::Optional(Some(value))) => {
            builder
                .store_bit(true)
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            store_body_value(builder, item_ty, value)
        }
        (AbiType::Array(_), _) => Err(AbiCodecError::UnsupportedType { ty: "array" }),
        (
            AbiType::Map {
                key,
                value: value_ty,
                key_bits,
            },
            AbiValue::Map(entries),
        ) => store_body_ref(builder, encode_map_cell(key, value_ty, *key_bits, entries)?),
        (AbiType::Unknown(_), _) => Err(AbiCodecError::UnsupportedType { ty: "unknown" }),
        (ty, value) => Err(AbiCodecError::TypeMismatch {
            expected: abi_type_name(ty),
            actual: abi_value_name(value),
        }),
    }
}

fn store_body_ref(builder: &mut Builder, cell: Arc<Cell>) -> Result<(), AbiCodecError> {
    builder
        .store_ref(cell)
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    Ok(())
}

pub(super) fn load_body_value(slice: &mut Slice, ty: &AbiType) -> Result<AbiValue, AbiCodecError> {
    match ty {
        AbiType::Int { bits } => {
            validate_integer_width_codec("int", *bits)?;
            let value = slice.load_big_int(usize::from(*bits)).map_err(|source| {
                AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                }
            })?;
            ensure_signed_range(&value, *bits)?;
            Ok(AbiValue::Int(value))
        }
        AbiType::Uint { bits } => {
            validate_integer_width_codec("uint", *bits)?;
            let value = slice.load_big_uint(usize::from(*bits)).map_err(|source| {
                AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                }
            })?;
            ensure_unsigned_range(&value, *bits)?;
            Ok(AbiValue::Uint(value))
        }
        AbiType::Bool => {
            slice
                .load_bit()
                .map(AbiValue::Bool)
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })
        }
        AbiType::Bytes => {
            let cell = load_body_ref(slice)?;
            Ok(AbiValue::Bytes(read_snake_cell(cell)?))
        }
        AbiType::String => {
            let cell = load_body_ref(slice)?;
            let bytes = read_snake_cell(cell)?;
            let value = String::from_utf8(bytes).map_err(|source| AbiCodecError::InvalidUtf8 {
                error: source.to_string(),
            })?;
            Ok(AbiValue::String(value))
        }
        AbiType::Address => {
            let address =
                MsgAddressInt::load_tlb(slice).map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            match address {
                MsgAddressInt::Std {
                    anycast: None,
                    address,
                } => Ok(AbiValue::Address(address)),
                MsgAddressInt::Std {
                    anycast: Some(_), ..
                } => Err(AbiCodecError::MalformedAddress {
                    reason: "standard address contains anycast".to_string(),
                }),
                MsgAddressInt::Var { .. } => Err(AbiCodecError::MalformedAddress {
                    reason: "variable-length internal addresses are unsupported".to_string(),
                }),
            }
        }
        AbiType::Cell => Ok(AbiValue::Cell(load_body_ref(slice)?)),
        AbiType::Slice => Ok(AbiValue::Slice(load_body_ref(slice)?)),
        AbiType::Tuple(fields) => {
            let values = fields
                .iter()
                .map(|field| load_body_value(slice, &field.ty))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(AbiValue::Tuple(values))
        }
        AbiType::Optional(item_ty) => {
            let has_value = slice
                .load_bit()
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
            if has_value {
                Ok(AbiValue::Optional(Some(Box::new(load_body_value(
                    slice, item_ty,
                )?))))
            } else {
                Ok(AbiValue::Optional(None))
            }
        }
        AbiType::Array(_) => Err(AbiCodecError::UnsupportedType { ty: "array" }),
        AbiType::Map {
            key,
            value,
            key_bits,
        } => {
            let cell = load_body_ref(slice)?;
            decode_map_cell(key, value, *key_bits, cell).map(AbiValue::Map)
        }
        AbiType::Unknown(_) => Err(AbiCodecError::UnsupportedType { ty: "unknown" }),
    }
}

fn load_body_ref(slice: &mut Slice) -> Result<Arc<Cell>, AbiCodecError> {
    slice
        .load_reference()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}
