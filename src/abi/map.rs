use super::*;

use std::sync::Arc;

use crate::tvm::{BitKey, Builder, Cell, HashmapE, Slice};

pub(super) fn encode_map_cell(
    key_ty: &AbiType,
    value_ty: &AbiType,
    explicit_key_bits: Option<u16>,
    entries: &[(AbiValue, AbiValue)],
) -> Result<Arc<Cell>, AbiCodecError> {
    let key_bits = map_key_bits(key_ty, explicit_key_bits)?;
    let mut dict = HashmapE::new(usize::from(key_bits));
    for (key, value) in entries {
        let bit_key = abi_map_key_to_bit_key(key_ty, key, key_bits)?;
        let value_cell = encode_map_value_cell(value_ty, value)?;
        if dict
            .insert_bit_key(bit_key.clone(), value_cell)
            .map_err(|source| AbiCodecError::MalformedBody {
                reason: source.to_string(),
            })?
            .is_some()
        {
            return Err(AbiCodecError::DuplicateMapKey {
                key_hex: hex::encode(bit_key.data()),
            });
        }
    }

    let mut builder = Builder::new();
    builder
        .store_hashmap_e_with(&dict, |builder, value| {
            builder.store_ref(value.clone())?;
            Ok(())
        })
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}

pub(super) fn decode_map_cell(
    key_ty: &AbiType,
    value_ty: &AbiType,
    explicit_key_bits: Option<u16>,
    cell: Arc<Cell>,
) -> Result<Vec<(AbiValue, AbiValue)>, AbiCodecError> {
    let key_bits = map_key_bits(key_ty, explicit_key_bits)?;
    let mut slice = Slice::new(cell);
    let dict = slice
        .load_hashmap_e_with(usize::from(key_bits), |slice| {
            slice.load_reference().map_err(Into::into)
        })
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    if !slice.is_empty() {
        return Err(AbiCodecError::MalformedBody {
            reason: format!(
                "map cell has trailing data: {} bits and {} refs",
                slice.remaining_bits(),
                slice.remaining_refs()
            ),
        });
    }

    dict.iter()
        .map(|(key, value_cell)| {
            Ok((
                bit_key_to_abi_map_key(key_ty, key)?,
                decode_map_value_cell(value_ty, value_cell.clone())?,
            ))
        })
        .collect()
}

fn encode_map_value_cell(value_ty: &AbiType, value: &AbiValue) -> Result<Arc<Cell>, AbiCodecError> {
    let mut builder = Builder::new();
    store_body_value(&mut builder, value_ty, value)?;
    builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })
}

fn decode_map_value_cell(value_ty: &AbiType, cell: Arc<Cell>) -> Result<AbiValue, AbiCodecError> {
    let mut slice = Slice::new(cell);
    let value = load_body_value(&mut slice, value_ty)?;
    if !slice.is_empty() {
        return Err(AbiCodecError::MalformedBody {
            reason: format!(
                "map value cell has trailing data: {} bits and {} refs",
                slice.remaining_bits(),
                slice.remaining_refs()
            ),
        });
    }
    Ok(value)
}

fn map_key_bits(key_ty: &AbiType, explicit_key_bits: Option<u16>) -> Result<u16, AbiCodecError> {
    let (kind, bits) = match key_ty {
        AbiType::Int { bits } => ("int", *bits),
        AbiType::Uint { bits } => ("uint", *bits),
        other => {
            return Err(AbiCodecError::UnsupportedMapKey {
                ty: abi_type_name(other),
            });
        }
    };
    validate_integer_width_codec(kind, bits)?;
    if let Some(actual) = explicit_key_bits {
        validate_integer_width_codec("map key", actual)?;
        if actual != bits {
            return Err(AbiCodecError::InvalidMapKeyBits {
                kind,
                expected: bits,
                actual,
            });
        }
    }
    Ok(bits)
}

fn abi_map_key_to_bit_key(
    key_ty: &AbiType,
    key: &AbiValue,
    key_bits: u16,
) -> Result<BitKey, AbiCodecError> {
    let mut builder = Builder::new();
    match (key_ty, key) {
        (AbiType::Int { bits }, AbiValue::Int(value)) => {
            validate_integer_width_codec("int", *bits)?;
            ensure_signed_range(value, *bits)?;
            builder
                .store_big_int(value, usize::from(key_bits))
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
        }
        (AbiType::Uint { bits }, AbiValue::Uint(value)) => {
            validate_integer_width_codec("uint", *bits)?;
            ensure_unsigned_range(value, *bits)?;
            builder
                .store_big_uint(value, usize::from(key_bits))
                .map_err(|source| AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                })?;
        }
        (other, _) if !matches!(other, AbiType::Int { .. } | AbiType::Uint { .. }) => {
            return Err(AbiCodecError::UnsupportedMapKey {
                ty: abi_type_name(other),
            });
        }
        (ty, value) => {
            return Err(AbiCodecError::TypeMismatch {
                expected: abi_type_name(ty),
                actual: abi_value_name(value),
            });
        }
    }
    let cell = builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    BitKey::new(cell.data().to_vec(), usize::from(key_bits)).map_err(|source| {
        AbiCodecError::MalformedBody {
            reason: source.to_string(),
        }
    })
}

fn bit_key_to_abi_map_key(key_ty: &AbiType, key: &BitKey) -> Result<AbiValue, AbiCodecError> {
    let mut builder = Builder::new();
    builder
        .store_bits(key.data(), key.bit_len())
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    let cell = builder
        .build()
        .map_err(|source| AbiCodecError::MalformedBody {
            reason: source.to_string(),
        })?;
    let mut slice = Slice::new(cell);
    match key_ty {
        AbiType::Int { bits } => {
            let value = slice.load_big_int(usize::from(*bits)).map_err(|source| {
                AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                }
            })?;
            Ok(AbiValue::Int(value))
        }
        AbiType::Uint { bits } => {
            let value = slice.load_big_uint(usize::from(*bits)).map_err(|source| {
                AbiCodecError::MalformedBody {
                    reason: source.to_string(),
                }
            })?;
            Ok(AbiValue::Uint(value))
        }
        other => Err(AbiCodecError::UnsupportedMapKey {
            ty: abi_type_name(other),
        }),
    }
}
