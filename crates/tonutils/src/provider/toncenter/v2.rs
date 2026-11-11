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
        Auth, HttpTransport, decode_boc_value, optional_int256, optional_str, parse_int256,
        required_i64, required_str, value_u64,
    },
    stack::{stack_from_json, stack_to_toncenter_v2},
    traits::{
        GetAccountStateExt, GetMasterchainInfoExt, GetTransactionsExt, RunGetMethodExt,
        SendExternalMessageExt,
    },
    types::{
        AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
        Transaction, TransactionList,
    },
};

const PROVIDER: &str = "ToncenterV2";
const MAINNET: &str = "https://toncenter.com/api/v2";
const TESTNET: &str = "https://testnet.toncenter.com/api/v2";

/// Toncenter API v2 provider.
#[derive(Clone, Debug)]
pub struct ToncenterV2 {
    transport: HttpTransport,
}

impl ToncenterV2 {
    pub fn mainnet() -> Self {
        Self::builder()
            .endpoint(MAINNET)
            .build()
            .expect("default Toncenter v2 endpoint is valid")
    }

    pub fn testnet() -> Self {
        Self::builder()
            .endpoint(TESTNET)
            .build()
            .expect("default Toncenter v2 endpoint is valid")
    }

    pub fn builder() -> ToncenterV2Builder {
        ToncenterV2Builder::default()
    }
}

/// Builder for [`ToncenterV2`].
pub struct ToncenterV2Builder {
    endpoint: String,
    api_key: Option<String>,
    timeout: Option<Duration>,
    client: Option<Client>,
}

impl Default for ToncenterV2Builder {
    fn default() -> Self {
        Self {
            endpoint: MAINNET.to_owned(),
            api_key: None,
            timeout: None,
            client: None,
        }
    }
}

impl ToncenterV2Builder {
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

    pub fn build(self) -> ProviderResult<ToncenterV2> {
        Ok(ToncenterV2 {
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
impl GetMasterchainInfoExt for ToncenterV2 {
    async fn get_masterchain_info(&mut self) -> ProviderResult<MasterchainInfo> {
        let request = self.transport.request(Method::GET, "getMasterchainInfo")?;
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let result = unwrap_envelope(&value)?;
        let last = result
            .get("last")
            .ok_or_else(|| invalid_response("missing `last` block"))?;
        Ok(MasterchainInfo {
            last: parse_block(last)?,
        })
    }
}

#[async_trait]
impl GetAccountStateExt for ToncenterV2 {
    async fn get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockId>,
    ) -> ProviderResult<AccountState> {
        let mut request = self
            .transport
            .request(Method::GET, "getAddressInformation")?
            .query(&[("address", account.to_raw())]);
        if let Some(block) = &block {
            request = request.query(&[("seqno", block.seqno.to_string())]);
        }
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let result = unwrap_envelope(&value)?;
        parse_account(&result)
    }
}

#[async_trait]
impl RunGetMethodExt for ToncenterV2 {
    async fn run_get_method(
        &mut self,
        account: Address,
        method: MethodRef<'_>,
        stack: TvmStack,
        block: Option<BlockId>,
    ) -> ProviderResult<RunMethodResult> {
        let method = match method {
            MethodRef::Name(name) => Value::String(name.to_owned()),
            MethodRef::Id(id) => Value::Number(id.into()),
        };
        let mut body = json!({
            "address": account.to_raw(),
            "method": method,
            "stack": stack_to_toncenter_v2(&stack)?,
        });
        if let Some(block) = &block {
            body["seqno"] = json!(block.seqno);
        }
        let request = self
            .transport
            .request(Method::POST, "runGetMethod")?
            .json(&body);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let result = unwrap_envelope(&value)?;
        let exit_code = required_i64(&result, "exit_code", PROVIDER)?;
        let exit_code =
            i32::try_from(exit_code).map_err(|error| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("exit_code does not fit in i32: {error}"),
            })?;
        let stack = result
            .get("stack")
            .ok_or_else(|| invalid_response("missing `stack` field"))?;
        Ok(RunMethodResult {
            exit_code,
            stack: stack_from_json(stack, PROVIDER)?,
            block,
        })
    }
}

#[async_trait]
impl SendExternalMessageExt for ToncenterV2 {
    async fn send_external_message_boc(
        &mut self,
        body: Vec<u8>,
    ) -> ProviderResult<SendMessageResult> {
        let request = self
            .transport
            .request(Method::POST, "sendBocReturnHash")?
            .json(&json!({ "boc": general_purpose::STANDARD.encode(body) }));
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let result = unwrap_envelope(&value)?;
        let message_hash = result
            .get("hash")
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
impl GetTransactionsExt for ToncenterV2 {
    async fn get_transactions(
        &mut self,
        count: u32,
        account: Address,
        lt: u64,
        hash: Int256,
    ) -> ProviderResult<TransactionList> {
        let request = self
            .transport
            .request(Method::GET, "getTransactions")?
            .query(&[
                ("address", account.to_raw()),
                ("limit", count.to_string()),
                ("lt", lt.to_string()),
                ("hash", hash.to_hex()),
            ]);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let result = unwrap_envelope(&value)?;
        let transactions = result
            .as_array()
            .ok_or_else(|| invalid_response("transactions result is not an array"))?
            .iter()
            .map(parse_transaction)
            .collect::<ProviderResult<Vec<_>>>()?;
        Ok(TransactionList { transactions })
    }
}

fn parse_block(value: &Value) -> ProviderResult<BlockId> {
    let workchain = required_i64(value, "workchain", PROVIDER)?;
    let shard = value_i64(value.get("shard"), "shard")?;
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
    let state = value
        .get("state")
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_response("missing account `state`"))?;
    let status = parse_status(state)?;
    let balance = value
        .get("balance")
        .map(parse_biguint_value)
        .transpose()?
        .unwrap_or_else(|| BigUint::from(0u8));
    let code = optional_cell(value.get("code"))?;
    let data = optional_cell(value.get("data"))?;
    let last_transaction = value.get("last_transaction_id");
    let last_transaction_lt = last_transaction
        .and_then(|value| value.get("lt"))
        .map(|value| value_u64(Some(value), "lt", PROVIDER))
        .transpose()?;
    let last_transaction_hash = last_transaction
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .map(parse_int256)
        .transpose()
        .map_err(|error| invalid_response(format!("invalid last transaction hash: {error}")))?;
    let frozen_hash = optional_int256(optional_str(value, "frozen_hash"))
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
    let transaction_id = value
        .get("transaction_id")
        .ok_or_else(|| invalid_response("missing `transaction_id`"))?;
    let lt = value_u64(transaction_id.get("lt"), "transaction_id.lt", PROVIDER)?;
    let hash = parse_int256(required_str(transaction_id, "hash", PROVIDER)?)
        .map_err(|error| invalid_response(format!("invalid transaction hash: {error}")))?;
    let utime = value
        .get("utime")
        .map(|value| value_u64(Some(value), "utime", PROVIDER))
        .transpose()?;
    let raw_boc = value
        .get("data")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(super::super::http::decode_bytes)
        .transpose()
        .map_err(|error| invalid_response(format!("invalid transaction BoC: {error}")))?;
    Ok(Transaction {
        lt,
        hash,
        utime,
        raw_boc,
    })
}

fn unwrap_envelope(value: &Value) -> ProviderResult<Value> {
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = value
            .get("error")
            .map(ToString::to_string)
            .unwrap_or_else(|| "request failed".to_owned());
        return Err(ProviderError::Api {
            provider: PROVIDER,
            status: 200,
            message,
        });
    }
    Ok(value
        .get("result")
        .cloned()
        .unwrap_or_else(|| value.clone()))
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

fn value_i64(value: Option<&Value>, field: &str) -> ProviderResult<i64> {
    let Some(value) = value else {
        return Err(invalid_response(format!("missing `{field}`")));
    };
    if let Some(value) = value.as_i64() {
        return Ok(value);
    }
    if let Some(value) = value.as_str() {
        return value
            .parse::<i64>()
            .map_err(|error| invalid_response(format!("invalid `{field}`: {error}")));
    }
    Err(invalid_response(format!("`{field}` is not an integer")))
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

    use super::{parse_account, parse_transaction, unwrap_envelope};

    #[test]
    fn unwraps_toncenter_error_envelope() {
        let error = unwrap_envelope(&json!({"ok": false, "error": "bad request"}))
            .expect_err("error envelope");
        assert!(error.to_string().contains("bad request"));
    }

    #[test]
    fn parses_account_and_transaction() {
        let account = parse_account(&json!({
            "state": "active",
            "balance": "1000",
            "last_transaction_id": {
                "lt": "9",
                "hash": "0000000000000000000000000000000000000000000000000000000000000004"
            }
        }))
        .expect("valid account");
        assert_eq!(account.balance.to_string(), "1000");
        assert_eq!(account.last_transaction_lt, Some(9));

        let transaction = parse_transaction(&json!({
            "transaction_id": {
                "lt": "9",
                "hash": "0000000000000000000000000000000000000000000000000000000000000004"
            },
            "utime": 1700000000
        }))
        .expect("valid transaction");
        assert_eq!(transaction.lt, 9);
        assert_eq!(transaction.utime, Some(1_700_000_000));
    }
}
