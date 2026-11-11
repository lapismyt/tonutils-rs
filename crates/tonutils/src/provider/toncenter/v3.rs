use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use num_bigint::BigUint;
use reqwest::{Client, Method};
use serde_json::{Value, json};

use crate::{
    tl::TlInt256 as Int256,
    tlb::AccountStatus,
    tvm::{Address, Cell, TvmStack},
};

use crate::provider::{
    error::{ProviderError, ProviderResult},
    http::{
        Auth, HttpTransport, decode_boc_value, optional_int256, parse_int256, required_i64,
        required_str, required_u64, value_u64,
    },
    stack::{stack_from_json, stack_to_toncenter_v3},
    traits::{
        GetAccountState, GetMasterchainInfo, GetTransactions, RunGetMethod, SendExternalMessage,
    },
    types::{
        AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
        Transaction, TransactionList,
    },
};

const PROVIDER: &str = "ToncenterV3";
const MAINNET: &str = "https://toncenter.com/api/v3";
const TESTNET: &str = "https://testnet.toncenter.com/api/v3";

/// Toncenter API v3 provider.
#[derive(Clone, Debug)]
pub struct ToncenterV3 {
    transport: HttpTransport,
}

impl ToncenterV3 {
    pub fn mainnet() -> Self {
        Self::builder()
            .endpoint(MAINNET)
            .build()
            .expect("default Toncenter v3 endpoint is valid")
    }

    pub fn testnet() -> Self {
        Self::builder()
            .endpoint(TESTNET)
            .build()
            .expect("default Toncenter v3 endpoint is valid")
    }

    pub fn builder() -> ToncenterV3Builder {
        ToncenterV3Builder::default()
    }
}

/// Builder for [`ToncenterV3`].
pub struct ToncenterV3Builder {
    endpoint: String,
    api_key: Option<String>,
    timeout: Option<Duration>,
    client: Option<Client>,
}

impl Default for ToncenterV3Builder {
    fn default() -> Self {
        Self {
            endpoint: MAINNET.to_owned(),
            api_key: None,
            timeout: None,
            client: None,
        }
    }
}

impl ToncenterV3Builder {
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    #[must_use]
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn build(self) -> ProviderResult<ToncenterV3> {
        Ok(ToncenterV3 {
            transport: HttpTransport::new(
                &self.endpoint,
                self.api_key,
                Auth::Header("x-api-key"),
                self.timeout,
                self.client,
            )?,
        })
    }
}

#[async_trait]
impl GetMasterchainInfo for ToncenterV3 {
    async fn get_masterchain_info(&mut self) -> ProviderResult<MasterchainInfo> {
        let request = self.transport.request(Method::GET, "masterchainInfo")?;
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let value = unwrap_envelope(value);
        let last = value
            .get("last")
            .ok_or_else(|| invalid_response("missing `last` block"))?;
        Ok(MasterchainInfo {
            last: parse_block(last)?,
        })
    }
}

#[async_trait]
impl GetAccountState for ToncenterV3 {
    async fn get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockId>,
    ) -> ProviderResult<AccountState> {
        if block.is_some() {
            return Err(ProviderError::UnsupportedBlock { provider: PROVIDER });
        }
        let request = self
            .transport
            .request(Method::GET, "accountStates")?
            .query(&[
                ("address", account.to_raw()),
                ("include_boc", "true".to_owned()),
            ]);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let value = unwrap_envelope(value);
        let account = value
            .get("accounts")
            .and_then(Value::as_array)
            .and_then(|accounts| accounts.first())
            .ok_or_else(|| invalid_response("accountStates returned no accounts"))?;
        parse_account(account)
    }
}

#[async_trait]
impl RunGetMethod for ToncenterV3 {
    async fn run_get_method(
        &mut self,
        account: Address,
        method: MethodRef<'_>,
        stack: TvmStack,
        block: Option<BlockId>,
    ) -> ProviderResult<RunMethodResult> {
        if block.is_some() {
            return Err(ProviderError::UnsupportedBlock { provider: PROVIDER });
        }
        let method = match method {
            MethodRef::Name(name) => name.to_owned(),
            MethodRef::Id(id) => id.to_string(),
        };
        let body = json!({
            "address": account.to_raw(),
            "method": method,
            "stack": stack_to_toncenter_v3(&stack)?,
        });
        let request = self
            .transport
            .request(Method::POST, "runGetMethod")?
            .json(&body);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let value = unwrap_envelope(value);
        let exit_code = required_i64(&value, "exit_code", PROVIDER)?;
        let exit_code =
            i32::try_from(exit_code).map_err(|error| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("exit_code does not fit in i32: {error}"),
            })?;
        let stack = value
            .get("stack")
            .ok_or_else(|| invalid_response("missing `stack` field"))?;
        Ok(RunMethodResult {
            exit_code,
            stack: stack_from_json(stack, PROVIDER)?,
            block: None,
        })
    }
}

#[async_trait]
impl SendExternalMessage for ToncenterV3 {
    async fn send_external_message_boc(
        &mut self,
        body: Vec<u8>,
    ) -> ProviderResult<SendMessageResult> {
        let request = self
            .transport
            .request(Method::POST, "message")?
            .json(&json!({ "boc": general_purpose::STANDARD.encode(body) }));
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let value = unwrap_envelope(value);
        let message_hash = value
            .get("message_hash")
            .and_then(Value::as_str)
            .map(parse_int256)
            .transpose()
            .map_err(|error| invalid_response(format!("invalid message hash: {error}")))?;
        Ok(SendMessageResult {
            status_code: None,
            message_hash,
        })
    }
}

#[async_trait]
impl GetTransactions for ToncenterV3 {
    async fn get_transactions(
        &mut self,
        count: u32,
        account: Address,
        lt: u64,
        _hash: Int256,
    ) -> ProviderResult<TransactionList> {
        let request = self
            .transport
            .request(Method::GET, "transactions")?
            .query(&[
                ("account", account.to_raw()),
                ("limit", count.to_string()),
                ("end_lt", lt.to_string()),
                ("sort", "desc".to_owned()),
            ]);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let value = unwrap_envelope(value);
        let transactions = value
            .get("transactions")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid_response("missing `transactions` array"))?
            .iter()
            .map(parse_transaction)
            .collect::<ProviderResult<Vec<_>>>()?;
        Ok(TransactionList { transactions })
    }
}

fn parse_block(value: &Value) -> ProviderResult<BlockId> {
    let workchain = required_i64(value, "workchain", PROVIDER)?;
    let shard = required_str(value, "shard", PROVIDER)?
        .parse::<i64>()
        .map_err(|error| invalid_response(format!("invalid shard: {error}")))?;
    let seqno = required_i64(value, "seqno", PROVIDER)?;
    Ok(BlockId {
        workchain: i32::try_from(workchain).map_err(|error| invalid_response(error.to_string()))?,
        shard,
        seqno: i32::try_from(seqno).map_err(|error| invalid_response(error.to_string()))?,
        root_hash: parse_int256(required_str(value, "root_hash", PROVIDER)?)
            .map_err(|error| invalid_response(format!("invalid root_hash: {error}")))?,
        file_hash: parse_int256(required_str(value, "file_hash", PROVIDER)?)
            .map_err(|error| invalid_response(format!("invalid file_hash: {error}")))?,
    })
}

fn parse_account(value: &Value) -> ProviderResult<AccountState> {
    let status = parse_status(required_str(value, "status", PROVIDER)?)?;
    let balance = value
        .get("balance")
        .map(parse_biguint_value)
        .transpose()?
        .unwrap_or_else(|| BigUint::from(0u8));
    let code = optional_cell(value.get("code_boc"))?;
    let data = optional_cell(value.get("data_boc"))?;
    let last_transaction_lt = value
        .get("last_transaction_lt")
        .map(|value| value_u64(Some(value), "last_transaction_lt", PROVIDER))
        .transpose()?;
    let last_transaction_hash = value
        .get("last_transaction_hash")
        .and_then(Value::as_str)
        .map(parse_int256)
        .transpose()
        .map_err(|error| invalid_response(format!("invalid last transaction hash: {error}")))?;
    let frozen_hash = optional_int256(value.get("frozen_hash").and_then(Value::as_str))
        .map_err(|error| invalid_response(format!("invalid frozen_hash: {error}")))?;
    Ok(AccountState {
        status,
        balance,
        code,
        data,
        last_transaction_lt,
        last_transaction_hash,
        frozen_hash,
    })
}

fn optional_cell(value: Option<&Value>) -> ProviderResult<Option<std::sync::Arc<Cell>>> {
    let Some(value) = value.and_then(Value::as_str) else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    decode_boc_value(value)
        .map(Some)
        .map_err(|error| invalid_response(format!("invalid account BoC: {error}")))
}

fn parse_transaction(value: &Value) -> ProviderResult<Transaction> {
    Ok(Transaction {
        lt: required_u64(value, "lt", PROVIDER)?,
        hash: parse_int256(required_str(value, "hash", PROVIDER)?)
            .map_err(|error| invalid_response(format!("invalid transaction hash: {error}")))?,
        utime: value
            .get("now")
            .map(|value| value_u64(Some(value), "now", PROVIDER))
            .transpose()?,
        raw_boc: None,
    })
}

fn parse_status(value: &str) -> ProviderResult<AccountStatus> {
    match value {
        "active" => Ok(AccountStatus::Active),
        "uninit" | "uninitialized" => Ok(AccountStatus::Uninit),
        "frozen" => Ok(AccountStatus::Frozen),
        "nonexist" | "nonexistent" | "non_exist" | "none" => Ok(AccountStatus::Nonexist),
        other => Err(invalid_response(format!(
            "unknown account status `{other}`"
        ))),
    }
}

fn parse_biguint_value(value: &Value) -> ProviderResult<BigUint> {
    if let Some(value) = value.as_u64() {
        return Ok(BigUint::from(value));
    }
    if let Some(value) = value.as_str() {
        return super::super::http::parse_biguint(value)
            .map_err(|error| invalid_response(format!("invalid balance: {error}")));
    }
    Err(invalid_response("balance is not an integer"))
}

fn unwrap_envelope(value: Value) -> Value {
    value.get("result").cloned().unwrap_or(value)
}

fn invalid_response(message: impl Into<String>) -> ProviderError {
    ProviderError::InvalidResponse {
        provider: PROVIDER,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_account, parse_transaction};

    #[test]
    fn parses_account_state() {
        let account = parse_account(&json!({
            "status": "uninit",
            "balance": "55",
        }))
        .expect("valid account");
        assert_eq!(account.balance.to_string(), "55");
        assert!(account.code.is_none());
    }

    #[test]
    fn parses_transaction_summary() {
        let transaction = parse_transaction(&json!({
            "lt": 12,
            "hash": "0000000000000000000000000000000000000000000000000000000000000005",
            "now": 1_700_000_000
        }))
        .expect("valid transaction");
        assert_eq!(transaction.lt, 12);
        assert_eq!(transaction.utime, Some(1_700_000_000));
    }
}
