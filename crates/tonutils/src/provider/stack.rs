use base64::{Engine as _, engine::general_purpose};
use num_bigint::{BigInt, Sign};
use serde_json::{Value, json};

use crate::tvm::{TvmStack, TvmStackEntry, serialize_boc};

use super::{
    error::{ProviderError, ProviderResult},
    http::decode_boc_value,
};

pub(crate) fn stack_from_json(value: &Value, provider: &'static str) -> ProviderResult<TvmStack> {
    let entries = value
        .as_array()
        .ok_or_else(|| ProviderError::InvalidResponse {
            provider,
            message: "stack is not an array".to_owned(),
        })?
        .iter()
        .map(stack_entry_from_json)
        .collect::<ProviderResult<Vec<_>>>()?;
    Ok(TvmStack::new(entries))
}

fn stack_entry_from_json(value: &Value) -> ProviderResult<TvmStackEntry> {
    if let Some(parts) = value.as_array() {
        if parts.len() != 2 {
            return Err(invalid_stack("stack entry array must have two elements"));
        }
        let kind = parts[0]
            .as_str()
            .ok_or_else(|| invalid_stack("stack entry type is not a string"))?;
        return parse_entry(kind, &parts[1]);
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_stack("stack entry object has no string `type`"))?;
    let payload = value
        .get("value")
        .or_else(|| value.get(kind))
        .or_else(|| value.get("tuple"))
        .unwrap_or(&Value::Null);
    parse_entry(kind, payload)
}

fn parse_entry(kind: &str, payload: &Value) -> ProviderResult<TvmStackEntry> {
    match kind {
        "num" | "int" | "int257" | "tinyint" => {
            let value = payload
                .as_str()
                .ok_or_else(|| invalid_stack("numeric stack value is not a string"))?;
            Ok(TvmStackEntry::Int(parse_bigint(value)?))
        }
        "cell" => Ok(TvmStackEntry::Cell(parse_boc_payload(payload, "cell")?)),
        "slice" => Ok(TvmStackEntry::Slice(parse_boc_payload(payload, "slice")?)),
        "null" | "nan" => Ok(TvmStackEntry::Null),
        "tuple" => {
            let entries = payload
                .as_array()
                .ok_or_else(|| invalid_stack("tuple stack value is not an array"))?
                .iter()
                .map(stack_entry_from_json)
                .collect::<ProviderResult<Vec<_>>>()?;
            Ok(TvmStackEntry::Tuple(entries))
        }
        "list" => {
            let entries = payload
                .as_array()
                .ok_or_else(|| invalid_stack("list stack value is not an array"))?
                .iter()
                .map(stack_entry_from_json)
                .collect::<ProviderResult<Vec<_>>>()?;
            Ok(TvmStackEntry::List(entries))
        }
        "unsupported" => {
            let value = payload
                .as_str()
                .ok_or_else(|| invalid_stack("unsupported stack value is not a string"))?;
            Ok(TvmStackEntry::Unsupported(
                super::http::decode_bytes(value).map_err(invalid_stack)?,
            ))
        }
        other => Err(invalid_stack(format!("unknown stack entry type `{other}`"))),
    }
}

/// Toncenter v2 returns cell and slice values as `{"bytes": "<boc>"}` objects,
/// while Toncenter v3 and TonAPI return plain strings.
fn parse_boc_payload(
    payload: &Value,
    kind: &str,
) -> ProviderResult<std::sync::Arc<crate::tvm::Cell>> {
    let value = payload
        .as_str()
        .or_else(|| payload.get("bytes").and_then(Value::as_str))
        .ok_or_else(|| {
            invalid_stack(format!(
                "{kind} stack value is neither a string nor a `bytes` object"
            ))
        })?;
    decode_boc_value(value).map_err(invalid_stack)
}

fn parse_bigint(value: &str) -> ProviderResult<BigInt> {
    let value = value.trim();
    let (negative, value) = match value.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, value),
    };
    let parsed = if let Some(value) = value.strip_prefix("0x") {
        BigInt::parse_bytes(value.as_bytes(), 16)
    } else if value.len() > 1 && value.starts_with('0') && value.bytes().all(|b| b.is_ascii_digit())
    {
        BigInt::parse_bytes(value.as_bytes(), 10)
    } else {
        BigInt::parse_bytes(value.as_bytes(), 10)
            .or_else(|| BigInt::parse_bytes(value.as_bytes(), 16))
    }
    .ok_or_else(|| invalid_stack(format!("invalid integer `{value}`")))?;
    Ok(if negative { -parsed } else { parsed })
}

fn invalid_stack(reason: impl Into<String>) -> ProviderError {
    ProviderError::InvalidStack {
        reason: reason.into(),
    }
}

pub(crate) fn stack_to_toncenter_v2(stack: &TvmStack) -> ProviderResult<Vec<Value>> {
    stack.entries().iter().map(entry_to_toncenter_v2).collect()
}

fn entry_to_toncenter_v2(entry: &TvmStackEntry) -> ProviderResult<Value> {
    match entry {
        TvmStackEntry::Null => Ok(json!(["null", ""])),
        TvmStackEntry::Int(value) => Ok(json!(["num", int_for_toncenter(value)])),
        TvmStackEntry::Cell(cell) => Ok(json!(["cell", encode_cell(cell)?])),
        TvmStackEntry::Slice(cell) => Ok(json!(["slice", encode_cell(cell)?])),
        TvmStackEntry::Tuple(entries) => Ok(json!([
            "tuple",
            entries
                .iter()
                .map(entry_to_toncenter_v2)
                .collect::<ProviderResult<Vec<_>>>()?
        ])),
        TvmStackEntry::List(entries) => Ok(json!([
            "list",
            entries
                .iter()
                .map(entry_to_toncenter_v2)
                .collect::<ProviderResult<Vec<_>>>()?
        ])),
        TvmStackEntry::Unsupported(_) => Err(ProviderError::UnsupportedStackEntry {
            kind: "unsupported",
        }),
    }
}

pub(crate) fn stack_to_toncenter_v3(stack: &TvmStack) -> ProviderResult<Vec<Value>> {
    stack
        .entries()
        .iter()
        .map(|entry| entry_to_object(entry, false))
        .collect()
}

pub(crate) fn stack_to_tonapi(stack: &TvmStack) -> ProviderResult<Vec<Value>> {
    stack
        .entries()
        .iter()
        .map(|entry| entry_to_object(entry, true))
        .collect()
}

fn entry_to_object(entry: &TvmStackEntry, tonapi: bool) -> ProviderResult<Value> {
    match entry {
        TvmStackEntry::Null => Ok(json!({"type": "null", "value": ""})),
        TvmStackEntry::Int(value) => {
            if tonapi {
                if let Ok(value) = i64::try_from(value.clone()) {
                    return Ok(json!({"type": "tinyint", "value": value.to_string()}));
                }
                return Ok(json!({"type": "int257", "value": int257_hex(value)}));
            }
            Ok(json!({"type": "num", "value": int_for_toncenter(value)}))
        }
        TvmStackEntry::Cell(cell) => Ok(json!({"type": "cell", "value": encode_cell(cell)?})),
        TvmStackEntry::Slice(cell) => Ok(json!({"type": "slice", "value": encode_cell(cell)?})),
        TvmStackEntry::Tuple(entries) => Ok(json!({
            "type": "tuple",
            "value": entries
                .iter()
                .map(|entry| entry_to_object(entry, tonapi))
                .collect::<ProviderResult<Vec<_>>>()?
        })),
        TvmStackEntry::List(_) => Err(ProviderError::UnsupportedStackEntry { kind: "list" }),
        TvmStackEntry::Unsupported(_) => Err(ProviderError::UnsupportedStackEntry {
            kind: "unsupported",
        }),
    }
}

fn encode_cell(cell: &std::sync::Arc<crate::tvm::Cell>) -> ProviderResult<String> {
    let bytes = serialize_boc(cell, true)?;
    Ok(general_purpose::STANDARD.encode(bytes))
}

fn int_for_toncenter(value: &BigInt) -> String {
    if value.sign() == Sign::Minus {
        value.to_string()
    } else {
        format!("0x{}", value.to_str_radix(16))
    }
}

fn int257_hex(value: &BigInt) -> String {
    let modulus = BigInt::from(1) << 257u32;
    let mut value = value % &modulus;
    if value.sign() == Sign::Minus {
        value += &modulus;
    }
    format!("0x{}", value.to_str_radix(16))
}

#[cfg(test)]
mod tests {
    use num_bigint::BigInt;
    use serde_json::json;

    use crate::tvm::{TvmStack, TvmStackEntry};

    use super::{stack_from_json, stack_to_tonapi, stack_to_toncenter_v2};

    #[test]
    fn parses_toncenter_array_stack() {
        let stack = stack_from_json(
            &json!([["num", "0x2a"], ["null", ""], ["tuple", [["num", "1"]]]]),
            "test",
        )
        .expect("valid stack");
        assert_eq!(
            stack.entries(),
            &[
                TvmStackEntry::Int(BigInt::from(42)),
                TvmStackEntry::Null,
                TvmStackEntry::Tuple(vec![TvmStackEntry::Int(BigInt::from(1))]),
            ]
        );
    }

    #[test]
    fn parses_toncenter_v2_cell_bytes_objects() {
        let boc = "te6cckEBAQEAJAAAQ4AMiB/HjSggcHLHKKLniWIo834XNprhIcsO73tLA4XzMxC89RGs";
        let stack = stack_from_json(
            &json!([
                ["cell", {"bytes": boc}],
                ["slice", {"bytes": boc}],
            ]),
            "test",
        )
        .expect("valid stack");
        assert!(matches!(stack.entries()[0], TvmStackEntry::Cell(_)));
        assert!(matches!(stack.entries()[1], TvmStackEntry::Slice(_)));
    }

    #[test]
    fn parses_object_stack() {
        let stack = stack_from_json(
            &json!([{"type": "num", "value": "7"}, {"type": "null", "value": ""}]),
            "test",
        )
        .expect("valid stack");
        assert_eq!(
            stack.entries(),
            &[TvmStackEntry::Int(BigInt::from(7)), TvmStackEntry::Null]
        );
    }

    #[test]
    fn serializes_toncenter_stack() {
        let stack = TvmStack::new(vec![
            TvmStackEntry::Int(BigInt::from(42)),
            TvmStackEntry::Int(BigInt::from(-42)),
        ]);
        assert_eq!(
            stack_to_toncenter_v2(&stack).expect("serializable"),
            vec![json!(["num", "0x2a"]), json!(["num", "-42"])]
        );
    }

    #[test]
    fn serializes_tonapi_small_int_as_tinyint() {
        let stack = TvmStack::new(vec![TvmStackEntry::Int(BigInt::from(42))]);
        assert_eq!(
            stack_to_tonapi(&stack).expect("serializable"),
            vec![json!({"type": "tinyint", "value": "42"})]
        );
    }
}
