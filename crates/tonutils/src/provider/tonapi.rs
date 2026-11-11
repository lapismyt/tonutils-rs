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

use super::{
    error::{ProviderError, ProviderResult},
    http::{
        Auth, HttpTransport, decode_boc_value, optional_int256, optional_str, parse_int256,
        required_i64, required_str, required_u64, value_u64,
    },
    stack::{stack_from_json, stack_to_tonapi},
    traits::{
        GetAccountStateExt, GetMasterchainInfoExt, GetTransactionsExt, RunGetMethodExt,
        SendExternalMessageExt,
    },
    types::{
        AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
        Transaction, TransactionList,
    },
};

const PROVIDER: &str = "TonAPI";
const MAINNET: &str = "https://tonapi.io";
const TESTNET: &str = "https://testnet.tonapi.io";

/// TonAPI provider.
#[derive(Clone, Debug)]
pub struct TonApi {
    transport: HttpTransport,
}

impl TonApi {
    pub fn mainnet() -> Self {
        Self::builder()
            .endpoint(MAINNET)
            .build()
            .expect("default TonAPI endpoint is valid")
    }

    pub fn testnet() -> Self {
        Self::builder()
            .endpoint(TESTNET)
            .build()
            .expect("default TonAPI endpoint is valid")
    }

    pub fn builder() -> TonApiBuilder {
        TonApiBuilder::default()
    }
}

/// Builder for [`TonApi`].
pub struct TonApiBuilder {
    endpoint: String,
    token: Option<String>,
    timeout: Option<Duration>,
    client: Option<Client>,
}

impl Default for TonApiBuilder {
    fn default() -> Self {
        Self {
            endpoint: MAINNET.to_owned(),
            token: None,
            timeout: None,
            client: None,
        }
    }
}

impl TonApiBuilder {
    #[must_use]
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
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

    pub fn build(self) -> ProviderResult<TonApi> {
        Ok(TonApi {
            transport: HttpTransport::new(
                &self.endpoint,
                self.token,
                Auth::Bearer,
                self.timeout,
                self.client,
            )?,
        })
    }
}

#[async_trait]
impl GetMasterchainInfoExt for TonApi {
    async fn get_masterchain_info(&mut self) -> ProviderResult<MasterchainInfo> {
        let request = self
            .transport
            .request(Method::GET, "v2/blockchain/masterchain-head")?;
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        Ok(MasterchainInfo {
            last: parse_block(&value)?,
        })
    }
}

#[async_trait]
impl GetAccountStateExt for TonApi {
    async fn get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockId>,
    ) -> ProviderResult<AccountState> {
        if block.is_some() {
            return Err(ProviderError::UnsupportedBlock { provider: PROVIDER });
        }
        let path = format!("v2/blockchain/accounts/{}", account.to_raw());
        let request = self.transport.request(Method::GET, &path)?;
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        parse_account(&value)
    }
}

#[async_trait]
impl RunGetMethodExt for TonApi {
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
        let MethodRef::Name(method) = method else {
            return Err(ProviderError::UnsupportedMethodRef { provider: PROVIDER });
        };
        let path = format!(
            "v2/blockchain/accounts/{}/methods/{method}",
            account.to_raw()
        );
        let request = self
            .transport
            .request(Method::POST, &path)?
            .json(&json!({ "args": stack_to_tonapi(&stack)? }));
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let exit_code = required_i64(&value, "exit_code", PROVIDER)?;
        let exit_code =
            i32::try_from(exit_code).map_err(|error| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("exit_code does not fit in i32: {error}"),
            })?;
        let stack = value
            .get("stack")
            .ok_or_else(|| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: "missing `stack` field".to_owned(),
            })?;
        Ok(RunMethodResult {
            exit_code,
            stack: stack_from_json(stack, PROVIDER)?,
            block: None,
        })
    }
}

#[async_trait]
impl SendExternalMessageExt for TonApi {
    async fn send_external_message_boc(
        &mut self,
        body: Vec<u8>,
    ) -> ProviderResult<SendMessageResult> {
        let request = self
            .transport
            .request(Method::POST, "v2/blockchain/message")?
            .json(&json!({ "boc": general_purpose::STANDARD.encode(body) }));
        self.transport.send_empty(request, PROVIDER).await?;
        Ok(SendMessageResult {
            status_code: None,
            message_hash: None,
        })
    }
}

#[async_trait]
impl GetTransactionsExt for TonApi {
    async fn get_transactions(
        &mut self,
        count: u32,
        account: Address,
        lt: u64,
        _hash: Int256,
    ) -> ProviderResult<TransactionList> {
        let path = format!("v2/blockchain/accounts/{}/transactions", account.to_raw());
        let request = self.transport.request(Method::GET, &path)?.query(&[
            ("before_lt", lt.to_string()),
            ("limit", count.to_string()),
            ("sort_order", "desc".to_owned()),
        ]);
        let value = self.transport.send_json_value(request, PROVIDER).await?;
        let transactions = value
            .get("transactions")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: "missing `transactions` array".to_owned(),
            })?
            .iter()
            .map(parse_transaction)
            .collect::<ProviderResult<Vec<_>>>()?;
        Ok(TransactionList { transactions })
    }
}

fn parse_block(value: &Value) -> ProviderResult<BlockId> {
    let workchain = value
        .get("workchain_id")
        .or_else(|| value.get("workchain"))
        .and_then(Value::as_i64)
        .ok_or_else(|| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: "missing `workchain_id`".to_owned(),
        })?;
    let shard = required_str(value, "shard", PROVIDER)?
        .parse::<i64>()
        .map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("invalid shard: {error}"),
        })?;
    let seqno = required_i64(value, "seqno", PROVIDER)?;
    Ok(BlockId {
        workchain: i32::try_from(workchain).map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("workchain does not fit in i32: {error}"),
        })?,
        shard,
        seqno: i32::try_from(seqno).map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("seqno does not fit in i32: {error}"),
        })?,
        root_hash: parse_int256(required_str(value, "root_hash", PROVIDER)?).map_err(|error| {
            ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("invalid root_hash: {error}"),
            }
        })?,
        file_hash: parse_int256(required_str(value, "file_hash", PROVIDER)?).map_err(|error| {
            ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("invalid file_hash: {error}"),
            }
        })?,
    })
}

fn parse_account(value: &Value) -> ProviderResult<AccountState> {
    let status = parse_status(required_str(value, "status", PROVIDER)?)?;
    let balance = value
        .get("balance")
        .map(parse_biguint_value)
        .transpose()?
        .unwrap_or_else(|| BigUint::from(0u8));
    let code = optional_cell(value.get("code"))?;
    let data = optional_cell(value.get("data"))?;
    let last_transaction_lt = optional_u64(value.get("last_transaction_lt"))?;
    let last_transaction_hash = optional_int256(optional_str(value, "last_transaction_hash"))
        .map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("invalid last_transaction_hash: {error}"),
        })?;
    let frozen_hash = optional_int256(optional_str(value, "frozen_hash")).map_err(|error| {
        ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("invalid frozen_hash: {error}"),
        }
    })?;
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
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    decode_boc_value(value)
        .map(Some)
        .map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("invalid account BoC: {error}"),
        })
}

fn parse_transaction(value: &Value) -> ProviderResult<Transaction> {
    Ok(Transaction {
        lt: required_u64(value, "lt", PROVIDER)?,
        hash: parse_int256(required_str(value, "hash", PROVIDER)?).map_err(|error| {
            ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("invalid transaction hash: {error}"),
            }
        })?,
        utime: optional_u64(value.get("now"))?,
        raw_boc: None,
    })
}

fn parse_status(value: &str) -> ProviderResult<AccountStatus> {
    match value {
        "active" => Ok(AccountStatus::Active),
        "uninit" | "uninitialized" => Ok(AccountStatus::Uninit),
        "frozen" => Ok(AccountStatus::Frozen),
        "nonexist" | "non_exist" | "none" => Ok(AccountStatus::Nonexist),
        other => Err(ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("unknown account status `{other}`"),
        }),
    }
}

fn parse_biguint_value(value: &Value) -> ProviderResult<BigUint> {
    if let Some(value) = value.as_u64() {
        return Ok(BigUint::from(value));
    }
    if let Some(value) = value.as_str() {
        return super::http::parse_biguint(value).map_err(|error| ProviderError::InvalidResponse {
            provider: PROVIDER,
            message: format!("invalid balance: {error}"),
        });
    }
    Err(ProviderError::InvalidResponse {
        provider: PROVIDER,
        message: "balance is not an integer".to_owned(),
    })
}

fn optional_u64(value: Option<&Value>) -> ProviderResult<Option<u64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value_u64(Some(value), "value", PROVIDER).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_account, parse_block};

    #[test]
    fn parses_masterchain_head() {
        let block = parse_block(&json!({
            "workchain_id": -1,
            "shard": "-9223372036854775808",
            "seqno": 42,
            "root_hash": "0000000000000000000000000000000000000000000000000000000000000001",
            "file_hash": "0000000000000000000000000000000000000000000000000000000000000002"
        }))
        .expect("valid block");
        assert_eq!(block.workchain, -1);
        assert_eq!(block.seqno, 42);
    }

    #[test]
    fn parses_account_summary() {
        let account = parse_account(&json!({
            "status": "active",
            "balance": "123",
            "last_transaction_lt": 7,
            "last_transaction_hash": "0000000000000000000000000000000000000000000000000000000000000003"
        }))
        .expect("valid account");
        assert_eq!(account.balance.to_string(), "123");
        assert_eq!(account.last_transaction_lt, Some(7));
        assert!(account.code.is_none());
    }
}
