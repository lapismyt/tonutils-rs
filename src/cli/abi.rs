use super::*;

use crate::abi::{
    AbiContract, AbiDefinition, AbiFunction, AbiFunctionKind, AbiParameter, AbiType, AbiValue,
    decode_get_method_outputs, encode_get_method_inputs, parse_abi_json_str,
};
use num_bigint::{BigUint, Sign};

#[derive(Debug, Serialize)]
pub(super) struct AbiRunGetMethodView {
    pub(super) block: BlockIdExtView,
    pub(super) shard_block: BlockIdExtView,
    pub(super) contract: String,
    pub(super) method: String,
    pub(super) method_id: u64,
    pub(super) exit_code: i32,
    pub(super) outputs: Vec<AbiNamedValueView>,
}

#[derive(Debug, Serialize)]
pub(super) struct AbiNamedValueView {
    pub(super) name: String,
    pub(super) abi_type: String,
    pub(super) value: Value,
}

pub(super) fn load_abi_file(path: &str) -> Result<AbiDefinition> {
    let json =
        fs::read_to_string(path).with_context(|| format!("failed to read ABI file {path}"))?;
    parse_abi_json_str(&json).with_context(|| format!("failed to parse ABI file {path}"))
}

pub(super) fn select_abi_contract<'a>(
    definition: &'a AbiDefinition,
    name: Option<&str>,
) -> Result<&'a AbiContract> {
    if let Some(name) = name {
        return definition
            .contracts
            .iter()
            .find(|contract| contract.name == name)
            .with_context(|| format!("ABI contract {name:?} was not found"));
    }
    if definition.contracts.len() != 1 {
        anyhow::bail!(
            "--contract is required when ABI file contains {} contracts",
            definition.contracts.len()
        );
    }
    Ok(&definition.contracts[0])
}

pub(super) fn select_abi_get_method<'a>(
    contract: &'a AbiContract,
    name: Option<&str>,
) -> Result<&'a AbiFunction> {
    if let Some(name) = name {
        let function = contract
            .methods
            .iter()
            .find(|method| method.name == name)
            .with_context(|| format!("ABI method {name:?} was not found"))?;
        if function.kind != AbiFunctionKind::GetMethod {
            anyhow::bail!("ABI method {name:?} is not a get-method");
        }
        return Ok(function);
    }

    let methods = contract
        .methods
        .iter()
        .filter(|method| method.kind == AbiFunctionKind::GetMethod)
        .collect::<Vec<_>>();
    if methods.len() != 1 {
        anyhow::bail!(
            "--method is required when ABI contract contains {} get-methods",
            methods.len()
        );
    }
    Ok(methods[0])
}

pub(super) fn abi_get_method_id(function: &AbiFunction) -> Result<u64> {
    match function.selector {
        crate::abi::AbiSelector::MethodId(method_id) => Ok(method_id),
        crate::abi::AbiSelector::None => Ok(crate::utils::method_name_to_id(&function.name)),
        selector => anyhow::bail!(
            "ABI get-method {} has invalid selector {selector:?}",
            function.name
        ),
    }
}

pub(super) fn parse_abi_named_args(
    parameters: &[AbiParameter],
    args: &[String],
) -> Result<Vec<AbiValue>> {
    let mut values = BTreeMap::<String, Value>::new();
    for arg in args {
        let (name, json) = arg
            .split_once('=')
            .with_context(|| format!("ABI argument {arg:?} must have format name=json"))?;
        if name.trim().is_empty() {
            anyhow::bail!("ABI argument name must not be empty");
        }
        if values
            .insert(name.to_owned(), serde_json::from_str(json)?)
            .is_some()
        {
            anyhow::bail!("ABI argument {name:?} was provided more than once");
        }
    }

    let decoded = parameters
        .iter()
        .map(|parameter| match values.remove(&parameter.name) {
            Some(value) => parse_abi_value(&parameter.ty, &value)
                .with_context(|| format!("failed to parse ABI argument {}", parameter.name)),
            None if parameter.optional || matches!(parameter.ty, AbiType::Optional(_)) => {
                parse_abi_value(&parameter.ty, &Value::Null)
                    .with_context(|| format!("failed to parse ABI argument {}", parameter.name))
            }
            None => anyhow::bail!("missing ABI argument {}", parameter.name),
        })
        .collect::<Result<Vec<_>>>()?;

    if let Some(name) = values.keys().next() {
        anyhow::bail!("unknown ABI argument {name}");
    }
    Ok(decoded)
}

pub(super) fn parse_abi_value(ty: &AbiType, value: &Value) -> Result<AbiValue> {
    match ty {
        AbiType::Int { .. } => parse_json_bigint(value).map(AbiValue::Int),
        AbiType::Uint { .. } => parse_json_biguint(value).map(AbiValue::Uint),
        AbiType::Bool => value
            .as_bool()
            .map(AbiValue::Bool)
            .context("expected JSON boolean"),
        AbiType::Bytes => parse_hex_json_string(value).map(AbiValue::Bytes),
        AbiType::String => value
            .as_str()
            .map(|value| AbiValue::String(value.to_owned()))
            .context("expected JSON string"),
        AbiType::Address => value
            .as_str()
            .context("expected address string")
            .and_then(|value| Address::from_str(value).context("invalid address"))
            .map(AbiValue::Address),
        AbiType::Cell => value
            .as_str()
            .context("expected BoC hex string")
            .and_then(parse_boc_hex_cell)
            .map(AbiValue::Cell),
        AbiType::Slice => value
            .as_str()
            .context("expected BoC hex string")
            .and_then(parse_boc_hex_cell)
            .map(AbiValue::Slice),
        AbiType::Tuple(fields) => parse_tuple_value(fields, value),
        AbiType::Array(item_ty) => value
            .as_array()
            .context("expected JSON array")?
            .iter()
            .map(|item| parse_abi_value(item_ty, item))
            .collect::<Result<Vec<_>>>()
            .map(AbiValue::Array),
        AbiType::Optional(item_ty) if value.is_null() => Ok(AbiValue::Optional(None)),
        AbiType::Optional(item_ty) => Ok(AbiValue::Optional(Some(Box::new(parse_abi_value(
            item_ty, value,
        )?)))),
        AbiType::Map { .. } => anyhow::bail!("ABI map/dictionary arguments are unsupported"),
        AbiType::Unknown(name) => anyhow::bail!("ABI unknown type {name:?} is unsupported"),
    }
}

pub(super) fn encode_abi_get_method_inputs(
    function: &AbiFunction,
    args: &[String],
) -> Result<Vec<TvmStackEntry>> {
    let values = parse_abi_named_args(&function.inputs, args)?;
    encode_get_method_inputs(function, &values).context("failed to encode ABI get-method inputs")
}

pub(super) fn abi_get_method_view(
    result: crate::tl::response::RunMethodResult,
    contract: &AbiContract,
    function: &AbiFunction,
    method_id: u64,
) -> Result<AbiRunGetMethodView> {
    if result.exit_code != 0 {
        anyhow::bail!("ABI get-method exited with code {}", result.exit_code);
    }
    let stack = result
        .decode_result_stack()
        .context("failed to decode ABI get-method stack")?
        .unwrap_or_else(TvmStack::empty);
    let values = decode_get_method_outputs(function, stack.entries())
        .context("failed to decode ABI get-method outputs")?;
    let outputs = function
        .outputs
        .iter()
        .zip(values.iter())
        .map(|(parameter, value)| {
            Ok(AbiNamedValueView {
                name: parameter.name.clone(),
                abi_type: abi_type_label(&parameter.ty),
                value: abi_value_json(value)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AbiRunGetMethodView {
        block: block_id_ext_view(&result.id),
        shard_block: block_id_ext_view(&result.shardblk),
        contract: contract.name.clone(),
        method: function.name.clone(),
        method_id,
        exit_code: result.exit_code,
        outputs,
    })
}

fn parse_tuple_value(fields: &[AbiParameter], value: &Value) -> Result<AbiValue> {
    let object = value.as_object().context("expected JSON object")?;
    let values = fields
        .iter()
        .map(|field| {
            object
                .get(&field.name)
                .with_context(|| format!("missing tuple field {}", field.name))
                .and_then(|value| parse_abi_value(&field.ty, value))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AbiValue::Tuple(values))
}

fn parse_json_bigint(value: &Value) -> Result<BigInt> {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            BigInt::parse_bytes(number.to_string().as_bytes(), 10).context("invalid integer")
        }
        Value::String(value) => parse_bigint_string(value),
        _ => anyhow::bail!("expected JSON integer number or decimal/hex string"),
    }
}

fn parse_json_biguint(value: &Value) -> Result<BigUint> {
    let value = parse_json_bigint(value)?;
    if value.sign() == Sign::Minus {
        anyhow::bail!("expected unsigned integer");
    }
    value.to_biguint().context("invalid unsigned integer")
}

fn parse_bigint_string(value: &str) -> Result<BigInt> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("integer string must not be empty");
    }
    let (negative, digits) = value
        .strip_prefix('-')
        .map_or((false, value), |digits| (true, digits));
    let parsed = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        BigInt::parse_bytes(hex.as_bytes(), 16)
    } else {
        BigInt::parse_bytes(digits.as_bytes(), 10)
    }
    .context("invalid integer string")?;
    Ok(if negative { -parsed } else { parsed })
}

fn parse_hex_json_string(value: &Value) -> Result<Vec<u8>> {
    let value = value.as_str().context("expected hex string")?;
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    hex::decode(hex).context("invalid hex string")
}

fn abi_value_json(value: &AbiValue) -> Result<Value> {
    Ok(match value {
        AbiValue::Int(value) => json!({ "decimal": value.to_str_radix(10) }),
        AbiValue::Uint(value) => json!({ "decimal": value.to_str_radix(10) }),
        AbiValue::Bool(value) => json!(value),
        AbiValue::Bytes(value) => json!({ "hex": hex::encode(value), "len": value.len() }),
        AbiValue::String(value) => json!(value),
        AbiValue::Address(value) => json!({
            "raw": value.to_raw(),
            "bounceable": value.to_bounceable(true),
            "non_bounceable": value.to_non_bounceable(true),
        }),
        AbiValue::Cell(cell) | AbiValue::Slice(cell) => {
            json!(raw_bytes_view(&crate::tvm::serialize_boc(cell, false)?))
        }
        AbiValue::Tuple(values) => json!(
            values
                .iter()
                .map(abi_value_json)
                .collect::<Result<Vec<_>>>()?
        ),
        AbiValue::Array(values) => json!(
            values
                .iter()
                .map(abi_value_json)
                .collect::<Result<Vec<_>>>()?
        ),
        AbiValue::Optional(None) => Value::Null,
        AbiValue::Optional(Some(value)) => abi_value_json(value)?,
    })
}

fn abi_type_label(ty: &AbiType) -> String {
    match ty {
        AbiType::Int { bits } => format!("int{bits}"),
        AbiType::Uint { bits } => format!("uint{bits}"),
        AbiType::Bool => "bool".to_owned(),
        AbiType::Bytes => "bytes".to_owned(),
        AbiType::String => "string".to_owned(),
        AbiType::Address => "address".to_owned(),
        AbiType::Cell => "cell".to_owned(),
        AbiType::Slice => "slice".to_owned(),
        AbiType::Tuple(fields) => format!(
            "tuple({})",
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, abi_type_label(&field.ty)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        AbiType::Array(item) => format!("array<{}>", abi_type_label(item)),
        AbiType::Map { key, value } => {
            format!("map<{},{}>", abi_type_label(key), abi_type_label(value))
        }
        AbiType::Optional(item) => format!("optional<{}>", abi_type_label(item)),
        AbiType::Unknown(value) => format!("unknown<{value}>"),
    }
}
