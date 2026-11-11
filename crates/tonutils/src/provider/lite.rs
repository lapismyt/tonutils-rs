//! Provider adapters for [`LiteClient`] and [`LiteBalancer`].
//!
//! These adapters talk directly to liteservers. Responses are returned as
//! received; no Merkle proof roots are validated here. Applications that need
//! proof validation should use the typed LiteClient helpers directly.

use async_trait::async_trait;
use num_bigint::BigUint;

use crate::{
    liteclient::{
        RUN_METHOD_MODE_RETURN_RESULT, RunMethodResultExt,
        balancer::LiteBalancer,
        boc::{DecodedAccountState, SimpleAccountState},
        client::LiteClient,
        tl::{LiteAccountId as AccountId, LiteBlockIdExt as BlockIdExt},
    },
    tl::TlInt256 as Int256,
    tlb::{
        Account as TlbAccount, AccountState as TlbAccountState, AccountStatus, TlbDeserialize,
        Transaction as TlbTransaction,
    },
    tvm::{Address, TvmStack, deserialize_boc_roots},
};

use super::{
    error::{ProviderError, ProviderResult},
    traits::{
        GetAccountState, GetMasterchainInfo, GetTransactions, RunGetMethod, SendExternalMessage,
    },
    types::{
        AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
        Transaction, TransactionList,
    },
};

macro_rules! impl_lite_provider {
    ($client:ty, $name:literal) => {
        #[async_trait]
        impl GetMasterchainInfo for $client {
            async fn get_masterchain_info(&mut self) -> ProviderResult<MasterchainInfo> {
                let info = <$client>::get_masterchain_info(self).await?;
                Ok(MasterchainInfo {
                    last: BlockId::from(info.last),
                })
            }
        }

        #[async_trait]
        impl GetAccountState for $client {
            async fn get_account_state(
                &mut self,
                account: Address,
                block: Option<BlockId>,
            ) -> ProviderResult<AccountState> {
                let decoded =
                    <$client>::get_account_state_typed(self, account, block.map(BlockIdExt::from))
                        .await?;
                Ok(account_state_from_decoded(decoded))
            }
        }

        #[async_trait]
        impl RunGetMethod for $client {
            async fn run_get_method(
                &mut self,
                account: Address,
                method: MethodRef<'_>,
                stack: TvmStack,
                block: Option<BlockId>,
            ) -> ProviderResult<RunMethodResult> {
                let block = match block {
                    Some(block) => BlockIdExt::from(block),
                    None => <$client>::get_masterchain_info(self).await?.last,
                };
                let result = <$client>::run_get_method(
                    self,
                    RUN_METHOD_MODE_RETURN_RESULT,
                    block,
                    account,
                    method.resolve_id(),
                    stack,
                )
                .await?;
                let stack = match result.result_stack_lossless() {
                    crate::liteclient::DecodedRunMethodResult::Missing => TvmStack::empty(),
                    crate::liteclient::DecodedRunMethodResult::Decoded(stack) => stack,
                    crate::liteclient::DecodedRunMethodResult::Undecodable { error, .. } => {
                        return Err(invalid(
                            $name,
                            format!("invalid get-method result BoC: {error}"),
                        ));
                    }
                };
                Ok(RunMethodResult {
                    exit_code: result.exit_code,
                    stack,
                    block: Some(BlockId::from(result.id)),
                })
            }
        }

        #[async_trait]
        impl SendExternalMessage for $client {
            async fn send_external_message_boc(
                &mut self,
                body: Vec<u8>,
            ) -> ProviderResult<SendMessageResult> {
                let status = <$client>::send_message(self, body).await?;
                Ok(SendMessageResult {
                    status_code: Some(status),
                    message_hash: None,
                })
            }
        }

        #[async_trait]
        impl GetTransactions for $client {
            async fn get_transactions(
                &mut self,
                count: u32,
                account: Address,
                lt: u64,
                hash: Int256,
            ) -> ProviderResult<TransactionList> {
                let account: AccountId = account.to_account_id();
                let response = <$client>::get_transactions(self, count, account, lt, hash).await?;
                transactions_from_boc(&response.transactions, $name)
            }
        }
    };
}

impl_lite_provider!(LiteClient, "LiteClient");
impl_lite_provider!(LiteBalancer, "LiteBalancer");

fn account_state_from_decoded(decoded: DecodedAccountState) -> AccountState {
    let simple = decoded.simple();
    let last_transaction_lt = simple.last_transaction_lt;
    let last_transaction_hash = simple.last_transaction_hash.map(Int256);
    match decoded.account {
        None => AccountState {
            status: AccountStatus::Nonexist,
            balance: BigUint::from(0u8),
            code: None,
            data: None,
            last_transaction_lt,
            last_transaction_hash,
            frozen_hash: None,
        },
        Some(TlbAccount::None) => AccountState {
            status: status_from_simple(&simple.state),
            balance: BigUint::from(0u8),
            code: None,
            data: None,
            last_transaction_lt,
            last_transaction_hash,
            frozen_hash: None,
        },
        Some(TlbAccount::Full { storage, .. }) => {
            let (status, code, data, frozen_hash) = match storage.state {
                TlbAccountState::Uninit => (AccountStatus::Uninit, None, None, None),
                TlbAccountState::Frozen { state_hash } => {
                    (AccountStatus::Frozen, None, None, Some(Int256(state_hash)))
                }
                TlbAccountState::Active { state_init } => (
                    AccountStatus::Active,
                    state_init.code,
                    state_init.data,
                    None,
                ),
            };
            AccountState {
                status,
                balance: storage.balance.grams.0,
                code,
                data,
                last_transaction_lt: Some(storage.last_trans_lt),
                last_transaction_hash,
                frozen_hash,
            }
        }
    }
}

fn status_from_simple(state: &SimpleAccountState) -> AccountStatus {
    match state {
        SimpleAccountState::None => AccountStatus::Nonexist,
        SimpleAccountState::Uninit => AccountStatus::Uninit,
        SimpleAccountState::Frozen => AccountStatus::Frozen,
        SimpleAccountState::Active => AccountStatus::Active,
    }
}

fn transactions_from_boc(raw: &[u8], provider: &'static str) -> ProviderResult<TransactionList> {
    if raw.is_empty() {
        return Ok(TransactionList::default());
    }
    let roots = deserialize_boc_roots(raw).map_err(|error| {
        invalid(
            provider,
            format!("failed to decode transactions BoC: {error}"),
        )
    })?;
    let transactions = roots
        .iter()
        .map(|root| {
            let hash = Int256(root.hash());
            TlbTransaction::from_cell(root.clone())
                .map(|transaction| (transaction, hash))
                .map_err(|error| {
                    invalid(provider, format!("failed to decode transaction: {error}"))
                })
        })
        .collect::<ProviderResult<Vec<_>>>()?
        .into_iter()
        .map(|(transaction, hash)| Transaction {
            lt: transaction.lt,
            hash,
            utime: Some(u64::from(transaction.now)),
            raw_boc: None,
        })
        .collect();
    Ok(TransactionList { transactions })
}

fn invalid(provider: &'static str, message: String) -> ProviderError {
    ProviderError::InvalidResponse { provider, message }
}
