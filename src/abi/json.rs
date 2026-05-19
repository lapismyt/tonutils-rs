use super::*;

use serde_json::{Map, Value};
use thiserror::Error;

/// Error returned when ABI JSON cannot be parsed into the local ABI model.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AbiJsonError {
    /// The JSON text is syntactically invalid.
    #[error("ABI JSON syntax error: {message}")]
    Syntax {
        /// Parser error message from `serde_json`.
        message: String,
    },
    /// A required field is absent.
    #[error("ABI JSON field {path} is required")]
    MissingField {
        /// JSON path for the missing field.
        path: String,
    },
    /// A field has a value kind that is not accepted by the ABI schema.
    #[error("ABI JSON field {path} expected {expected}, got {actual}")]
    InvalidType {
        /// JSON path for the invalid field.
        path: String,
        /// Expected JSON or ABI kind.
        expected: &'static str,
        /// Actual JSON kind.
        actual: &'static str,
    },
    /// A string field uses an unknown enum spelling.
    #[error("ABI JSON field {path} has unsupported value {value:?}")]
    UnsupportedValue {
        /// JSON path for the unsupported value.
        path: String,
        /// Rejected value.
        value: String,
    },
    /// Numeric value is outside the accepted range for the target field.
    #[error("ABI JSON field {path} value {value} is outside {expected}")]
    NumberOutOfRange {
        /// JSON path for the rejected number.
        path: String,
        /// Rejected number spelling.
        value: String,
        /// Accepted range.
        expected: &'static str,
    },
    /// A selector object contains more than one selector spelling.
    #[error("ABI JSON selector {path} is ambiguous: contains both method_id and opcode")]
    AmbiguousSelector {
        /// JSON path for the ambiguous selector object.
        path: String,
    },
    /// The document uses a known ABI compatibility shape this loader does not
    /// implement yet.
    #[error("ABI JSON field {path} uses unsupported compatibility shape {shape}")]
    UnsupportedShape {
        /// JSON path for the unsupported shape.
        path: String,
        /// Shape name or key set.
        shape: String,
    },
    /// The parsed model failed local ABI invariant validation.
    #[error("ABI JSON model validation failed: {source}")]
    Model {
        /// Validation error.
        source: AbiModelError,
    },
}

/// Parses an ABI JSON document into an [`AbiDefinition`].
///
/// The accepted schema is intentionally explicit and local to this crate:
/// top-level `name`, `version`, and `contracts`; contract `methods` and
/// `events`; function `kind`, `selector`, `inputs`, and `outputs`; parameter
/// `name`, `type`, and optional `optional`.
pub fn parse_abi_json_str(json: &str) -> Result<AbiDefinition, AbiJsonError> {
    let value = serde_json::from_str(json).map_err(|source| AbiJsonError::Syntax {
        message: source.to_string(),
    })?;
    parse_abi_json_value(&value)
}

/// Parses a pre-loaded JSON value into an [`AbiDefinition`].
pub fn parse_abi_json_value(value: &Value) -> Result<AbiDefinition, AbiJsonError> {
    let object = object_at(value, "$")?;
    reject_unsupported_document_shape(object)?;
    let definition = AbiDefinition {
        name: required_string(object, "name", "$.name")?.to_string(),
        version: required_string(object, "version", "$.version")?.to_string(),
        contracts: required_array(object, "contracts", "$.contracts")?
            .iter()
            .enumerate()
            .map(|(index, value)| parse_contract(value, &format!("$.contracts[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
    };
    definition
        .validate()
        .map_err(|source| AbiJsonError::Model { source })?;
    Ok(definition)
}

fn parse_contract(value: &Value, path: &str) -> Result<AbiContract, AbiJsonError> {
    let object = object_at(value, path)?;
    Ok(AbiContract {
        name: required_string(object, "name", &field_path(path, "name"))?.to_string(),
        methods: optional_array(object, "methods", &field_path(path, "methods"))?
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(index, value)| parse_function(value, &format!("{path}.methods[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
        events: optional_array(object, "events", &field_path(path, "events"))?
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(index, value)| parse_event(value, &format!("{path}.events[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_function(value: &Value, path: &str) -> Result<AbiFunction, AbiJsonError> {
    let object = object_at(value, path)?;
    Ok(AbiFunction {
        name: required_string(object, "name", &field_path(path, "name"))?.to_string(),
        kind: parse_function_kind(
            required_string(object, "kind", &field_path(path, "kind"))?,
            &field_path(path, "kind"),
        )?,
        selector: parse_selector(object.get("selector"), &field_path(path, "selector"))?,
        inputs: optional_array(object, "inputs", &field_path(path, "inputs"))?
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(index, value)| parse_parameter(value, &format!("{path}.inputs[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
        outputs: optional_array(object, "outputs", &field_path(path, "outputs"))?
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(index, value)| parse_parameter(value, &format!("{path}.outputs[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_event(value: &Value, path: &str) -> Result<AbiEvent, AbiJsonError> {
    let object = object_at(value, path)?;
    Ok(AbiEvent {
        name: required_string(object, "name", &field_path(path, "name"))?.to_string(),
        selector: parse_selector(object.get("selector"), &field_path(path, "selector"))?,
        fields: optional_array(object, "fields", &field_path(path, "fields"))?
            .unwrap_or(&[])
            .iter()
            .enumerate()
            .map(|(index, value)| parse_parameter(value, &format!("{path}.fields[{index}]")))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn parse_parameter(value: &Value, path: &str) -> Result<AbiParameter, AbiJsonError> {
    let object = object_at(value, path)?;
    let ty_path = field_path(path, "type");
    let ty = object
        .get("type")
        .ok_or_else(|| AbiJsonError::MissingField {
            path: ty_path.clone(),
        })
        .and_then(|value| parse_type(value, &ty_path))?;
    Ok(AbiParameter {
        name: required_string(object, "name", &field_path(path, "name"))?.to_string(),
        ty,
        optional: optional_bool(object, "optional", &field_path(path, "optional"))?
            .unwrap_or(false),
    })
}

fn parse_function_kind(value: &str, path: &str) -> Result<AbiFunctionKind, AbiJsonError> {
    match value {
        "get_method" | "get" => Ok(AbiFunctionKind::GetMethod),
        "internal_message" | "internal" => Ok(AbiFunctionKind::InternalMessage),
        "external_message" | "external" => Ok(AbiFunctionKind::ExternalMessage),
        value => Err(AbiJsonError::UnsupportedValue {
            path: path.to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_selector(value: Option<&Value>, path: &str) -> Result<AbiSelector, AbiJsonError> {
    let Some(value) = value else {
        return Ok(AbiSelector::None);
    };
    if value.is_null() {
        return Ok(AbiSelector::None);
    }
    let object = object_at(value, path)?;
    if object.contains_key("method_id") && object.contains_key("opcode") {
        return Err(AbiJsonError::AmbiguousSelector {
            path: path.to_string(),
        });
    }
    if let Some(value) = object.get("method_id") {
        return parse_u64(value, &field_path(path, "method_id")).map(AbiSelector::MethodId);
    }
    if let Some(value) = object.get("opcode") {
        return parse_u32(value, &field_path(path, "opcode")).map(AbiSelector::Opcode);
    }
    Err(AbiJsonError::MissingField {
        path: format!("{path}.method_id|opcode"),
    })
}

fn parse_type(value: &Value, path: &str) -> Result<AbiType, AbiJsonError> {
    match value {
        Value::String(value) => parse_type_string(value, path),
        Value::Object(object) => parse_type_object(object, path),
        value => Err(AbiJsonError::InvalidType {
            path: path.to_string(),
            expected: "string or object",
            actual: json_kind(value),
        }),
    }
}

fn parse_type_object(object: &Map<String, Value>, path: &str) -> Result<AbiType, AbiJsonError> {
    reject_unsupported_type_shape(object, path)?;
    if let Some(value) = object.get("tuple") {
        return Ok(AbiType::Tuple(
            array_at(value, &field_path(path, "tuple"))?
                .iter()
                .enumerate()
                .map(|(index, value)| parse_parameter(value, &format!("{path}.tuple[{index}]")))
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }
    if let Some(value) = object.get("array") {
        return Ok(AbiType::Array(Box::new(parse_type(
            value,
            &field_path(path, "array"),
        )?)));
    }
    if let Some(value) = object.get("optional") {
        return Ok(AbiType::Optional(Box::new(parse_type(
            value,
            &field_path(path, "optional"),
        )?)));
    }
    if let Some(value) = object.get("map") {
        let map = object_at(value, &field_path(path, "map"))?;
        let key_path = format!("{path}.map.key");
        let value_path = format!("{path}.map.value");
        let key_bits_path = format!("{path}.map.key_bits");
        let key = map
            .get("key")
            .ok_or_else(|| AbiJsonError::MissingField {
                path: key_path.clone(),
            })
            .and_then(|value| parse_type(value, &key_path))?;
        let value = map
            .get("value")
            .ok_or_else(|| AbiJsonError::MissingField {
                path: value_path.clone(),
            })
            .and_then(|value| parse_type(value, &value_path))?;
        let key_bits = map
            .get("key_bits")
            .map(|value| parse_u16(value, &key_bits_path))
            .transpose()?;
        return Ok(AbiType::Map {
            key: Box::new(key),
            value: Box::new(value),
            key_bits,
        });
    }
    if let Some(value) = object.get("unknown") {
        return Ok(AbiType::Unknown(
            string_at(value, &field_path(path, "unknown"))?.to_string(),
        ));
    }
    Err(AbiJsonError::MissingField {
        path: format!("{path}.tuple|array|optional|map|unknown"),
    })
}

fn reject_unsupported_document_shape(object: &Map<String, Value>) -> Result<(), AbiJsonError> {
    for key in [
        "ABI version",
        "functions",
        "getters",
        "receivers",
        "types",
        "data",
    ] {
        if object.contains_key(key) {
            return Err(AbiJsonError::UnsupportedShape {
                path: format!("$.{key}"),
                shape: key.to_string(),
            });
        }
    }
    Ok(())
}

fn reject_unsupported_type_shape(
    object: &Map<String, Value>,
    path: &str,
) -> Result<(), AbiJsonError> {
    for key in ["kind", "components", "fields", "format"] {
        if object.contains_key(key) {
            return Err(AbiJsonError::UnsupportedShape {
                path: field_path(path, key),
                shape: key.to_string(),
            });
        }
    }
    Ok(())
}

fn parse_type_string(value: &str, path: &str) -> Result<AbiType, AbiJsonError> {
    let value = value.trim();
    if let Some(inner) = value
        .strip_prefix("optional<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Ok(AbiType::Optional(Box::new(parse_type_string(inner, path)?)));
    }
    if let Some(inner) = value
        .strip_prefix("array<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return Ok(AbiType::Array(Box::new(parse_type_string(inner, path)?)));
    }
    if let Some(bits) = value.strip_prefix("uint") {
        return parse_bits(bits, path).map(|bits| AbiType::Uint { bits });
    }
    if let Some(bits) = value.strip_prefix("int") {
        return parse_bits(bits, path).map(|bits| AbiType::Int { bits });
    }
    match value {
        "bool" => Ok(AbiType::Bool),
        "bytes" => Ok(AbiType::Bytes),
        "string" => Ok(AbiType::String),
        "address" => Ok(AbiType::Address),
        "cell" => Ok(AbiType::Cell),
        "slice" => Ok(AbiType::Slice),
        value => Err(AbiJsonError::UnsupportedValue {
            path: path.to_string(),
            value: value.to_string(),
        }),
    }
}

fn parse_bits(value: &str, path: &str) -> Result<u16, AbiJsonError> {
    let bits = value
        .parse::<u16>()
        .map_err(|_| AbiJsonError::UnsupportedValue {
            path: path.to_string(),
            value: value.to_string(),
        })?;
    validate_integer_width("integer", bits).map_err(|source| AbiJsonError::Model { source })?;
    Ok(bits)
}

fn object_at<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, AbiJsonError> {
    value.as_object().ok_or_else(|| AbiJsonError::InvalidType {
        path: path.to_string(),
        expected: "object",
        actual: json_kind(value),
    })
}

fn array_at<'a>(value: &'a Value, path: &str) -> Result<&'a [Value], AbiJsonError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| AbiJsonError::InvalidType {
            path: path.to_string(),
            expected: "array",
            actual: json_kind(value),
        })
}

fn string_at<'a>(value: &'a Value, path: &str) -> Result<&'a str, AbiJsonError> {
    value.as_str().ok_or_else(|| AbiJsonError::InvalidType {
        path: path.to_string(),
        expected: "string",
        actual: json_kind(value),
    })
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<&'a str, AbiJsonError> {
    object
        .get(field)
        .ok_or_else(|| AbiJsonError::MissingField {
            path: path.to_string(),
        })
        .and_then(|value| string_at(value, path))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<&'a [Value], AbiJsonError> {
    object
        .get(field)
        .ok_or_else(|| AbiJsonError::MissingField {
            path: path.to_string(),
        })
        .and_then(|value| array_at(value, path))
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<&'a [Value]>, AbiJsonError> {
    object
        .get(field)
        .map(|value| array_at(value, path))
        .transpose()
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<bool>, AbiJsonError> {
    object
        .get(field)
        .map(|value| {
            value.as_bool().ok_or_else(|| AbiJsonError::InvalidType {
                path: path.to_string(),
                expected: "boolean",
                actual: json_kind(value),
            })
        })
        .transpose()
}

fn parse_u64(value: &Value, path: &str) -> Result<u64, AbiJsonError> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| AbiJsonError::NumberOutOfRange {
                path: path.to_string(),
                value: number.to_string(),
                expected: "u64",
            }),
        Value::String(value) => parse_numeric_string(value, path, "u64", u64::MAX),
        value => Err(AbiJsonError::InvalidType {
            path: path.to_string(),
            expected: "number or string",
            actual: json_kind(value),
        }),
    }
}

fn parse_u32(value: &Value, path: &str) -> Result<u32, AbiJsonError> {
    let parsed = parse_u64(value, path)?;
    u32::try_from(parsed).map_err(|_| AbiJsonError::NumberOutOfRange {
        path: path.to_string(),
        value: parsed.to_string(),
        expected: "u32",
    })
}

fn parse_u16(value: &Value, path: &str) -> Result<u16, AbiJsonError> {
    let parsed = parse_u64(value, path)?;
    u16::try_from(parsed).map_err(|_| AbiJsonError::NumberOutOfRange {
        path: path.to_string(),
        value: parsed.to_string(),
        expected: "u16",
    })
}

fn parse_numeric_string(
    value: &str,
    path: &str,
    expected: &'static str,
    max: u64,
) -> Result<u64, AbiJsonError> {
    let parsed = if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse::<u64>()
    }
    .map_err(|_| AbiJsonError::UnsupportedValue {
        path: path.to_string(),
        value: value.to_string(),
    })?;
    if parsed > max {
        return Err(AbiJsonError::NumberOutOfRange {
            path: path.to_string(),
            value: value.to_string(),
            expected,
        });
    }
    Ok(parsed)
}

fn field_path(path: &str, field: &str) -> String {
    format!("{path}.{field}")
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
