//! Typed emulation results and response parsing.

use std::sync::Arc;

use crate::tlb::{OutList, ShardAccount, TlbDeserialize, Transaction};
use crate::tvm::{Cell, VmStack, base64_to_boc};

use super::error::EmulatorError;

/// VM log verbosity passed when creating an emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum VmLogVerbosity {
    /// Keep only the last 256 characters of the VM log.
    Truncated = 0,
    /// Keep the complete VM log.
    Unlimited = 1,
    /// Include a cell hash and offset for every command.
    WithCellHash = 2,
    /// Include every stack value for every command.
    WithStack = 3,
}

impl VmLogVerbosity {
    /// Returns the upstream numeric verbosity value.
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Successful transaction emulation.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionEmulationResult {
    /// Emulated transaction.
    pub transaction: Transaction,
    /// Updated shard account after the transaction.
    pub shard_account: ShardAccount,
    /// VM execution log.
    pub vm_log: String,
    /// Compute-phase output actions, when the library produced a list.
    pub actions: Option<OutList>,
    /// Wall-clock emulation duration in seconds.
    pub elapsed_time: f64,
}

/// Successful get-method execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetMethodResult {
    /// Result stack produced by the method.
    pub stack: VmStack,
    /// Gas consumed by the method.
    pub gas_used: u64,
    /// TVM exit code.
    pub vm_exit_code: i32,
    /// VM execution log.
    pub vm_log: String,
    /// Library hash required to continue execution, when missing.
    pub missing_library: Option<[u8; 32]>,
}

/// Successful external or internal message emulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageEmulationResult {
    /// Code cell after execution.
    pub new_code: Arc<Cell>,
    /// Data cell after execution.
    pub new_data: Arc<Cell>,
    /// Whether the smart contract accepted the message.
    pub accepted: bool,
    /// TVM exit code.
    pub vm_exit_code: i32,
    /// VM execution log.
    pub vm_log: String,
    /// Gas consumed during execution.
    pub gas_used: u64,
    /// Compute-phase output actions, when the library produced a list.
    pub actions: Option<OutList>,
    /// Library hash required to continue execution, when missing.
    pub missing_library: Option<[u8; 32]>,
}

pub(super) fn parse_transaction_result(
    value: &serde_json::Value,
) -> Result<TransactionEmulationResult, EmulatorError> {
    ensure_success(value)?;
    let transaction_cell = decode_cell(value, "transaction")?;
    let shard_account_cell = decode_cell(value, "shard_account")?;
    Ok(TransactionEmulationResult {
        transaction: Transaction::from_cell(transaction_cell)?,
        shard_account: ShardAccount::from_cell(shard_account_cell)?,
        vm_log: field_str(value, "vm_log")?.to_owned(),
        actions: decode_actions(value)?,
        elapsed_time: field_f64(value, "elapsed_time")?,
    })
}

pub(super) fn parse_get_method_result(
    value: &serde_json::Value,
) -> Result<GetMethodResult, EmulatorError> {
    ensure_success(value)?;
    let stack_cell = decode_cell(value, "stack")?;
    Ok(GetMethodResult {
        stack: VmStack::from_cell(stack_cell)?,
        gas_used: field_u64(value, "gas_used")?,
        vm_exit_code: field_i32(value, "vm_exit_code")?,
        vm_log: field_str(value, "vm_log")?.to_owned(),
        missing_library: field_missing_library(value)?,
    })
}

pub(super) fn parse_message_result(
    value: &serde_json::Value,
) -> Result<MessageEmulationResult, EmulatorError> {
    ensure_success(value)?;
    Ok(MessageEmulationResult {
        new_code: decode_cell(value, "new_code")?,
        new_data: decode_cell(value, "new_data")?,
        accepted: field_bool(value, "accepted")?,
        vm_exit_code: field_i32(value, "vm_exit_code")?,
        vm_log: field_str(value, "vm_log")?.to_owned(),
        gas_used: field_u64(value, "gas_used")?,
        actions: decode_actions(value)?,
        missing_library: field_missing_library(value)?,
    })
}

fn ensure_success(value: &serde_json::Value) -> Result<(), EmulatorError> {
    if value
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(());
    }

    Err(EmulatorError::Emulation {
        message: value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("libemulator reported an unspecified error")
            .to_owned(),
        external_not_accepted: value
            .get("external_not_accepted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        vm_exit_code: value
            .get("vm_exit_code")
            .and_then(serde_json::Value::as_i64)
            .and_then(|code| i32::try_from(code).ok()),
        vm_log: value
            .get("vm_log")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        elapsed_time: value
            .get("elapsed_time")
            .and_then(serde_json::Value::as_f64),
    })
}

fn decode_cell(value: &serde_json::Value, field: &str) -> Result<Arc<Cell>, EmulatorError> {
    Ok(base64_to_boc(field_str(value, field)?)?)
}

fn decode_actions(value: &serde_json::Value) -> Result<Option<OutList>, EmulatorError> {
    let Some(encoded) = optional_str(value, "actions")? else {
        return Ok(None);
    };
    let cell = base64_to_boc(encoded)?;
    Ok(Some(OutList::from_cell(cell)?))
}

fn field_str<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str, EmulatorError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| EmulatorError::InvalidResponse(format!("missing string field `{field}`")))
}

fn optional_str<'a>(
    value: &'a serde_json::Value,
    field: &str,
) -> Result<Option<&'a str>, EmulatorError> {
    match value.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(EmulatorError::InvalidResponse(format!(
            "field `{field}` must be a string or null"
        ))),
    }
}

fn field_bool(value: &serde_json::Value, field: &str) -> Result<bool, EmulatorError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| EmulatorError::InvalidResponse(format!("missing boolean field `{field}`")))
}

fn field_i32(value: &serde_json::Value, field: &str) -> Result<i32, EmulatorError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| {
            EmulatorError::InvalidResponse(format!("missing integer field `{field}`"))
        })?;
    i32::try_from(number).map_err(|_| {
        EmulatorError::InvalidResponse(format!("field `{field}` does not fit a 32-bit integer"))
    })
}

fn field_u64(value: &serde_json::Value, field: &str) -> Result<u64, EmulatorError> {
    match value.get(field) {
        Some(serde_json::Value::String(text)) => text.parse::<u64>().map_err(|_| {
            EmulatorError::InvalidResponse(format!("field `{field}` is not an unsigned integer"))
        }),
        Some(serde_json::Value::Number(number)) => number.as_u64().ok_or_else(|| {
            EmulatorError::InvalidResponse(format!("field `{field}` is not an unsigned integer"))
        }),
        _ => Err(EmulatorError::InvalidResponse(format!(
            "missing unsigned integer field `{field}`"
        ))),
    }
}

fn field_f64(value: &serde_json::Value, field: &str) -> Result<f64, EmulatorError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| EmulatorError::InvalidResponse(format!("missing number field `{field}`")))
}

fn field_missing_library(value: &serde_json::Value) -> Result<Option<[u8; 32]>, EmulatorError> {
    let Some(encoded) = optional_str(value, "missing_library")? else {
        return Ok(None);
    };
    let mut hash = [0u8; 32];
    hex::decode_to_slice(encoded, &mut hash).map_err(|error| {
        EmulatorError::InvalidResponse(format!("invalid missing_library hash: {error}"))
    })?;
    Ok(Some(hash))
}
