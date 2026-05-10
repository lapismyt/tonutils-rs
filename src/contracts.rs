//! High-level smart-contract helpers built on LiteAPI calls.

use async_trait::async_trait;

use crate::liteclient::boc::{DecodedAccountState, SimpleAccount};
use crate::liteclient::{balancer::LiteBalancer, client::LiteClient};
use crate::tl::{
    BlockIdExt,
    common::{AccountId, Int256},
    response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
};
use crate::tlb::{CurrencyCollection, StateInit, TlbSerialize};
use crate::tvm::{Address, Cell, TvmStack, TvmStackEntry, deserialize_boc};
use std::borrow::Cow;
use std::sync::Arc;

/// Errors returned by high-level contract helpers that decode account state or
/// get-method stack values.
#[derive(Debug, thiserror::Error)]
pub enum ContractError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("contract provider error: {0}")]
    Provider(#[source] E),
    #[error("get-method exited with code {exit_code}")]
    NonZeroExitCode { exit_code: i32 },
    #[error("failed to decode contract data: {0}")]
    Decode(#[source] anyhow::Error),
    #[error("account is not active")]
    MissingActiveState,
    #[error("active account has no code")]
    MissingCode,
    #[error("active account has no data")]
    MissingData,
}

/// Errors returned while building a contract address from fixed code and typed
/// TL-B data.
#[derive(Debug, thiserror::Error)]
pub enum ContractBuildError {
    #[error("failed to decode contract code BoC: {0}")]
    InvalidCodeBoc(#[source] anyhow::Error),
    #[error("failed to serialize contract data as TL-B: {0}")]
    DataSerialization(#[source] crate::tlb::TlbError),
    #[error("failed to serialize contract StateInit: {0}")]
    StateInitSerialization(#[source] anyhow::Error),
    #[error("invalid contract derive configuration: {0}")]
    InvalidDeriveConfiguration(&'static str),
}

impl<E> ContractError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn provider(error: E) -> Self {
        Self::Provider(error)
    }

    pub fn decode(error: impl Into<anyhow::Error>) -> Self {
        Self::Decode(error.into())
    }
}

/// LiteAPI operations required by the generic contract wrapper.
#[async_trait]
pub trait ContractProvider: Send {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error>;

    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error>;

    async fn get_account_state_typed(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<DecodedAccountState, Self::Error>;

    async fn get_account_state_simple(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<SimpleAccount, Self::Error>;

    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error>;

    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error>;

    async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, Self::Error>;
}

#[async_trait]
impl ContractProvider for LiteClient {
    type Error = crate::liteclient::types::LiteError;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        LiteClient::get_masterchain_info(self).await
    }

    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        LiteClient::get_account_state(self, block, account).await
    }

    async fn get_account_state_typed(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<DecodedAccountState, Self::Error> {
        LiteClient::get_account_state_typed(self, account, Some(block)).await
    }

    async fn get_account_state_simple(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<SimpleAccount, Self::Error> {
        Ok(
            LiteClient::get_account_state_typed(self, account, Some(block))
                .await?
                .simple(),
        )
    }

    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        LiteClient::run_get_method(self, mode, block, account, method_id, stack).await
    }

    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
        LiteClient::send_message(self, body).await
    }

    async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        LiteClient::get_transactions(self, count, account, lt, hash).await
    }
}

#[async_trait]
impl ContractProvider for LiteBalancer {
    type Error = crate::liteclient::balancer::BalancerError;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        LiteBalancer::get_masterchain_info(self).await
    }

    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        LiteBalancer::get_account_state(self, block, account).await
    }

    async fn get_account_state_typed(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<DecodedAccountState, Self::Error> {
        LiteBalancer::get_account_state_typed(self, account, Some(block)).await
    }

    async fn get_account_state_simple(
        &mut self,
        block: BlockIdExt,
        account: Address,
    ) -> Result<SimpleAccount, Self::Error> {
        Ok(
            LiteBalancer::get_account_state_typed(self, account, Some(block))
                .await?
                .simple(),
        )
    }

    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        LiteBalancer::run_get_method(self, mode, block, account, method_id, stack).await
    }

    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
        LiteBalancer::send_message(self, body).await
    }

    async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        LiteBalancer::get_transactions(self, count, account, lt, hash).await
    }
}

/// A smart contract bound to an address and a LiteAPI provider.
pub struct Contract<'a, P: ContractProvider + ?Sized> {
    provider: &'a mut P,
    address: Address,
}

impl<'a, P: ContractProvider + ?Sized> Contract<'a, P> {
    pub fn new(provider: &'a mut P, address: Address) -> Self {
        Self { provider, address }
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    pub async fn get_state(&mut self, block: BlockIdExt) -> Result<AccountState, P::Error> {
        self.provider
            .get_account_state(block, self.address.to_account_id())
            .await
    }

    pub async fn get_state_latest(&mut self) -> Result<AccountState, P::Error> {
        let block = self.provider.get_masterchain_info().await?.last;
        self.get_state(block).await
    }

    pub async fn get_state_decoded(
        &mut self,
        block: BlockIdExt,
    ) -> Result<DecodedAccountState, P::Error> {
        self.provider
            .get_account_state_typed(block, self.address.clone())
            .await
    }

    pub async fn get_state_decoded_latest(&mut self) -> Result<DecodedAccountState, P::Error> {
        let block = self.provider.get_masterchain_info().await?.last;
        self.get_state_decoded(block).await
    }

    pub async fn get_state_simple(&mut self, block: BlockIdExt) -> Result<SimpleAccount, P::Error> {
        self.provider
            .get_account_state_simple(block, self.address.clone())
            .await
    }

    pub async fn get_state_simple_latest(&mut self) -> Result<SimpleAccount, P::Error> {
        let block = self.provider.get_masterchain_info().await?.last;
        self.get_state_simple(block).await
    }

    pub async fn active_state(
        &mut self,
        block: BlockIdExt,
    ) -> Result<StateInit, ContractError<P::Error>> {
        active_state_init(
            &self
                .get_state_decoded(block)
                .await
                .map_err(ContractError::provider)?,
        )
    }

    pub async fn active_state_latest(&mut self) -> Result<StateInit, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.active_state(block).await
    }

    pub async fn balance(
        &mut self,
        block: BlockIdExt,
    ) -> Result<CurrencyCollection, ContractError<P::Error>> {
        active_balance(
            &self
                .get_state_decoded(block)
                .await
                .map_err(ContractError::provider)?,
        )
    }

    pub async fn balance_latest(&mut self) -> Result<CurrencyCollection, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.balance(block).await
    }

    pub async fn code(&mut self, block: BlockIdExt) -> Result<Arc<Cell>, ContractError<P::Error>> {
        active_state_init(
            &self
                .get_state_decoded(block)
                .await
                .map_err(ContractError::provider)?,
        )?
        .code
        .ok_or(ContractError::MissingCode)
    }

    pub async fn code_latest(&mut self) -> Result<Arc<Cell>, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.code(block).await
    }

    pub async fn data(&mut self, block: BlockIdExt) -> Result<Arc<Cell>, ContractError<P::Error>> {
        active_state_init(
            &self
                .get_state_decoded(block)
                .await
                .map_err(ContractError::provider)?,
        )?
        .data
        .ok_or(ContractError::MissingData)
    }

    pub async fn data_latest(&mut self) -> Result<Arc<Cell>, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.data(block).await
    }

    pub async fn run_get_method(
        &mut self,
        block: BlockIdExt,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        self.provider
            .run_get_method(0, block, self.address.clone(), method_id, stack)
            .await
    }

    pub async fn run_get_method_by_name(
        &mut self,
        block: BlockIdExt,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        self.run_get_method(block, crate::utils::method_name_to_id(method_name), stack)
            .await
    }

    pub async fn run_get_method_latest(
        &mut self,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        let block = self.provider.get_masterchain_info().await?.last;
        self.run_get_method(block, method_id, stack).await
    }

    pub async fn run_get_method_by_name_latest(
        &mut self,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        self.run_get_method_latest(crate::utils::method_name_to_id(method_name), stack)
            .await
    }

    pub async fn run_get_method_typed(
        &mut self,
        block: BlockIdExt,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError<P::Error>> {
        decode_success_stack(
            self.run_get_method(block, method_id, stack)
                .await
                .map_err(ContractError::provider)?,
        )
    }

    pub async fn run_get_method_by_name_typed(
        &mut self,
        block: BlockIdExt,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError<P::Error>> {
        self.run_get_method_typed(block, crate::utils::method_name_to_id(method_name), stack)
            .await
    }

    pub async fn run_get_method_typed_latest(
        &mut self,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.run_get_method_typed(block, method_id, stack).await
    }

    pub async fn run_get_method_by_name_typed_latest(
        &mut self,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError<P::Error>> {
        self.run_get_method_typed_latest(crate::utils::method_name_to_id(method_name), stack)
            .await
    }

    pub async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, P::Error> {
        self.provider.send_external_message_boc(body).await
    }

    pub async fn get_transactions(
        &mut self,
        count: u32,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, P::Error> {
        self.provider
            .get_transactions(count, self.address.to_account_id(), lt, hash)
            .await
    }
}

/// A contract definition that derives its `StateInit` and address from fixed
/// code BoC bytes plus typed TL-B data.
pub trait ContractBlueprint {
    /// Typed contract data stored as `StateInit.data`.
    type Data: TlbSerialize;

    /// Returns the typed contract data value.
    fn data(&self) -> &Self::Data;

    /// Returns a BoC containing the contract code root cell.
    fn code_boc(&self) -> Cow<'static, [u8]>;

    /// Returns the workchain used for the derived address.
    fn workchain(&self) -> i8 {
        0
    }

    /// Builds the canonical `StateInit` for this blueprint.
    fn state_init(&self) -> Result<StateInit, ContractBuildError> {
        let code = deserialize_boc(&self.code_boc()).map_err(ContractBuildError::InvalidCodeBoc)?;
        let data = self
            .data()
            .to_cell()
            .map_err(ContractBuildError::DataSerialization)?;
        Ok(StateInit {
            code: Some(code),
            data: Some(data),
            ..StateInit::empty()
        })
    }

    /// Derives the standard contract address from this blueprint's
    /// `StateInit`.
    fn address(&self) -> Result<Address, ContractBuildError> {
        let state_init = self.state_init()?;
        address_from_state_init(self.workchain(), &state_init)
            .map_err(ContractBuildError::StateInitSerialization)
    }

    /// Binds this blueprint's derived address to an existing provider.
    fn bind<'a, P: ContractProvider + ?Sized>(
        &self,
        provider: &'a mut P,
    ) -> Result<Contract<'a, P>, ContractBuildError> {
        Ok(Contract::new(provider, self.address()?))
    }
}

/// Derives the standard contract address from a serialized `StateInit` cell.
pub fn address_from_state_init(
    workchain: i8,
    state_init: &StateInit,
) -> Result<Address, anyhow::Error> {
    let cell = state_init.to_cell()?;
    Ok(Address::new(workchain, cell.hash()))
}

fn decode_success_stack<E>(result: RunMethodResult) -> Result<Vec<TvmStackEntry>, ContractError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    if result.exit_code != 0 {
        return Err(ContractError::NonZeroExitCode {
            exit_code: result.exit_code,
        });
    }
    let stack = result
        .decode_result_stack()
        .map_err(ContractError::decode)?
        .unwrap_or_else(TvmStack::empty);
    Ok(stack.entries().to_vec())
}

fn active_state_init<E>(decoded: &DecodedAccountState) -> Result<StateInit, ContractError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match decoded.account.as_ref() {
        Some(crate::tlb::Account::Full { storage, .. }) => match &storage.state {
            crate::tlb::AccountState::Active { state_init } => Ok(state_init.clone()),
            crate::tlb::AccountState::Uninit | crate::tlb::AccountState::Frozen { .. } => {
                Err(ContractError::MissingActiveState)
            }
        },
        Some(crate::tlb::Account::None) | None => Err(ContractError::MissingActiveState),
    }
}

fn active_balance<E>(decoded: &DecodedAccountState) -> Result<CurrencyCollection, ContractError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
{
    match decoded.account.as_ref() {
        Some(crate::tlb::Account::Full { storage, .. }) => Ok(storage.balance.clone()),
        Some(crate::tlb::Account::None) | None => Err(ContractError::MissingActiveState),
    }
}

/// Lossless decoded view of optional `liteServer.runMethodResult.result`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedRunMethodResult {
    Missing,
    Decoded(TvmStack),
    Undecodable { raw: Vec<u8>, error: String },
}

pub trait RunMethodResultExt {
    fn raw_result_boc(&self) -> Option<&[u8]>;
    fn decode_result_stack(&self) -> anyhow::Result<Option<TvmStack>>;
    fn result_stack_lossless(&self) -> DecodedRunMethodResult;
}

impl RunMethodResultExt for RunMethodResult {
    fn raw_result_boc(&self) -> Option<&[u8]> {
        self.result.as_deref()
    }

    fn decode_result_stack(&self) -> anyhow::Result<Option<TvmStack>> {
        self.result.as_deref().map(TvmStack::from_boc).transpose()
    }

    fn result_stack_lossless(&self) -> DecodedRunMethodResult {
        match self.result.as_deref() {
            None => DecodedRunMethodResult::Missing,
            Some(raw) => match TvmStack::from_boc(raw) {
                Ok(stack) => DecodedRunMethodResult::Decoded(stack),
                Err(error) => DecodedRunMethodResult::Undecodable {
                    raw: raw.to_vec(),
                    error: error.to_string(),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tl::Int256;
    use crate::tlb::{
        AccountStorage, CurrencyCollection, Grams, MsgAddressInt, StorageExtraInfo, StorageInfo,
        StorageUsed,
    };
    use crate::tvm::Builder;
    use crate::tvm::TvmStackEntry;
    use num_bigint::BigUint;

    #[derive(Debug, thiserror::Error)]
    #[error("mock provider error")]
    struct MockError;

    struct MockProvider {
        latest: BlockIdExt,
        account: Address,
        raw_state: AccountState,
        state_calls: usize,
        method_calls: Vec<u64>,
        sent_messages: Vec<Vec<u8>>,
        transaction_calls: usize,
        exit_code: i32,
    }

    #[async_trait]
    impl ContractProvider for MockProvider {
        type Error = MockError;

        async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
            Ok(MasterchainInfo {
                last: self.latest.clone(),
                state_root_hash: Int256([1; 32]),
                init: crate::tl::common::ZeroStateIdExt {
                    workchain: -1,
                    root_hash: Int256([2; 32]),
                    file_hash: Int256([3; 32]),
                },
            })
        }

        async fn get_account_state(
            &mut self,
            block: BlockIdExt,
            account: AccountId,
        ) -> Result<AccountState, Self::Error> {
            assert_eq!(block, self.latest);
            assert_eq!(account, self.account.to_account_id());
            self.state_calls += 1;
            Ok(self.raw_state.clone())
        }

        async fn get_account_state_typed(
            &mut self,
            block: BlockIdExt,
            account: Address,
        ) -> Result<DecodedAccountState, Self::Error> {
            DecodedAccountState::from_raw(
                self.get_account_state(block, account.to_account_id())
                    .await?,
            )
            .map_err(|_| MockError)
        }

        async fn get_account_state_simple(
            &mut self,
            block: BlockIdExt,
            account: Address,
        ) -> Result<SimpleAccount, Self::Error> {
            Ok(self.get_account_state_typed(block, account).await?.simple())
        }

        async fn run_get_method(
            &mut self,
            _mode: u32,
            block: BlockIdExt,
            account: Address,
            method_id: u64,
            _stack: TvmStack,
        ) -> Result<RunMethodResult, Self::Error> {
            assert_eq!(block, self.latest);
            assert_eq!(account, self.account);
            self.method_calls.push(method_id);
            Ok(RunMethodResult {
                mode: (),
                id: self.latest.clone(),
                shardblk: self.latest.clone(),
                shard_proof: None,
                proof: None,
                state_proof: None,
                init_c7: None,
                lib_extras: None,
                exit_code: self.exit_code,
                result: Some(TvmStack::new(vec![TvmStackEntry::int(7)]).to_boc().unwrap()),
            })
        }

        async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
            self.sent_messages.push(body);
            Ok(1)
        }

        async fn get_transactions(
            &mut self,
            _count: u32,
            account: AccountId,
            _lt: u64,
            _hash: Int256,
        ) -> Result<TransactionList, Self::Error> {
            assert_eq!(account, self.account.to_account_id());
            self.transaction_calls += 1;
            Ok(TransactionList {
                ids: Vec::new(),
                transactions: Vec::new(),
            })
        }
    }

    fn block(seqno: i32) -> BlockIdExt {
        BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno,
            root_hash: Int256([4; 32]),
            file_hash: Int256([5; 32]),
        }
    }

    fn raw_account_state(account: crate::tlb::Account) -> AccountState {
        AccountState {
            id: block(1),
            shardblk: block(1),
            shard_proof: Vec::new(),
            proof: Vec::new(),
            state: crate::tvm::serialize_boc(&account.to_cell().unwrap(), false).unwrap(),
        }
    }

    fn mock_provider(
        address: Address,
        latest: BlockIdExt,
        account: crate::tlb::Account,
    ) -> MockProvider {
        MockProvider {
            latest,
            account: address,
            raw_state: raw_account_state(account),
            state_calls: 0,
            method_calls: Vec::new(),
            sent_messages: Vec::new(),
            transaction_calls: 0,
            exit_code: 0,
        }
    }

    fn active_account(address: Address, state_init: StateInit, grams: u64) -> crate::tlb::Account {
        crate::tlb::Account::Full {
            addr: MsgAddressInt::std(address),
            storage_stat: StorageInfo {
                used: StorageUsed::new(BigUint::from(1u8), BigUint::from(64u8)),
                last_paid: 1_700_000_000,
                due_payment: None,
                extra: StorageExtraInfo::None,
            },
            storage: AccountStorage {
                last_trans_lt: 42,
                balance: CurrencyCollection::grams(Grams::from(grams)),
                state: crate::tlb::AccountState::Active { state_init },
            },
        }
    }

    #[tokio::test]
    async fn contract_uses_provider_for_latest_state() {
        let address = Address::new(0, [9; 32]);
        let latest = block(42);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);

        let mut contract = Contract::new(&mut provider, address);
        let state = contract.get_state_latest().await.unwrap();

        assert!(!state.state.is_empty());
        assert_eq!(provider.state_calls, 1);
    }

    #[tokio::test]
    async fn contract_routes_method_name_to_id() {
        let address = Address::new(0, [8; 32]);
        let latest = block(43);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);

        let mut contract = Contract::new(&mut provider, address);
        let result = contract
            .run_get_method_by_name_latest("seqno", TvmStack::empty())
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            provider.method_calls,
            vec![crate::utils::method_name_to_id("seqno")]
        );
    }

    #[tokio::test]
    async fn decoded_and_simple_state_helpers_preserve_account_data() {
        let address = Address::new(0, [7; 32]);
        let latest = block(44);
        let account = active_account(address.clone(), StateInit::empty(), 123_456);
        let mut provider = mock_provider(address.clone(), latest, account.clone());

        let mut contract = Contract::new(&mut provider, address);
        let decoded = contract.get_state_decoded_latest().await.unwrap();
        let simple = contract.get_state_simple_latest().await.unwrap();

        assert_eq!(decoded.account, Some(account));
        assert_eq!(
            simple.state,
            crate::liteclient::boc::SimpleAccountState::Active
        );
        assert_eq!(simple.last_transaction_lt, Some(42));
    }

    #[tokio::test]
    async fn active_account_helpers_extract_balance_code_and_data() {
        let address = Address::new(0, [6; 32]);
        let latest = block(45);
        let code = Builder::new().build().unwrap();
        let data = Builder::new().build().unwrap();
        let state_init = StateInit {
            code: Some(code.clone()),
            data: Some(data.clone()),
            ..StateInit::empty()
        };
        let account = active_account(address.clone(), state_init.clone(), 77);
        let mut provider = mock_provider(address.clone(), latest, account);

        let mut contract = Contract::new(&mut provider, address);

        assert_eq!(contract.active_state_latest().await.unwrap(), state_init);
        assert_eq!(
            contract.balance_latest().await.unwrap().grams,
            Grams::from(77)
        );
        assert_eq!(contract.code_latest().await.unwrap().hash(), code.hash());
        assert_eq!(contract.data_latest().await.unwrap().hash(), data.hash());
    }

    #[tokio::test]
    async fn missing_active_state_and_code_data_are_documented_errors() {
        let address = Address::new(0, [5; 32]);
        let latest = block(46);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let mut contract = Contract::new(&mut provider, address);

        assert!(matches!(
            contract.active_state_latest().await,
            Err(ContractError::MissingActiveState)
        ));

        let address = Address::new(0, [4; 32]);
        let latest = block(47);
        let account = active_account(address.clone(), StateInit::empty(), 1);
        let mut provider = mock_provider(address.clone(), latest, account);
        let mut contract = Contract::new(&mut provider, address);

        assert!(matches!(
            contract.code_latest().await,
            Err(ContractError::MissingCode)
        ));
        assert!(matches!(
            contract.data_latest().await,
            Err(ContractError::MissingData)
        ));
    }

    #[tokio::test]
    async fn typed_get_method_decodes_stack_and_reports_non_zero_exit() {
        let address = Address::new(0, [3; 32]);
        let latest = block(48);
        let mut provider =
            mock_provider(address.clone(), latest.clone(), crate::tlb::Account::None);
        let mut contract = Contract::new(&mut provider, address.clone());

        assert_eq!(
            contract
                .run_get_method_by_name_typed_latest("seqno", TvmStack::empty())
                .await
                .unwrap(),
            vec![TvmStackEntry::int(7)]
        );

        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        provider.exit_code = 13;
        let mut contract = Contract::new(&mut provider, address);
        assert!(matches!(
            contract
                .run_get_method_typed_latest(1, TvmStack::empty())
                .await,
            Err(ContractError::NonZeroExitCode { exit_code: 13 })
        ));
    }

    #[tokio::test]
    async fn external_boc_and_transaction_helpers_route_through_provider() {
        let address = Address::new(0, [2; 32]);
        let latest = block(49);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let mut contract = Contract::new(&mut provider, address);

        assert_eq!(
            contract
                .send_external_message_boc(vec![1, 2, 3])
                .await
                .unwrap(),
            1
        );
        let _ = contract
            .get_transactions(4, 10, Int256([0xaa; 32]))
            .await
            .unwrap();

        assert_eq!(provider.sent_messages, vec![vec![1, 2, 3]]);
        assert_eq!(provider.transaction_calls, 1);
    }

    #[test]
    fn state_init_address_derivation_uses_serialized_cell_hash() {
        let code = Builder::new().build().unwrap();
        let state_init = StateInit {
            code: Some(code),
            ..StateInit::empty()
        };

        let address = address_from_state_init(0, &state_init).unwrap();
        assert_eq!(address.hash_part, state_init.to_cell().unwrap().hash());
        assert_eq!(address.workchain, 0);
    }

    struct BlueprintData {
        value: u32,
    }

    impl TlbSerialize for BlueprintData {
        fn store_tlb(&self, builder: &mut Builder) -> crate::tlb::Result<()> {
            builder.store_uint::<u32>(self.value as u32)?;
            Ok(())
        }
    }

    struct TestBlueprint {
        code_boc: Vec<u8>,
        data: BlueprintData,
        workchain: i8,
    }

    impl ContractBlueprint for TestBlueprint {
        type Data = BlueprintData;

        fn data(&self) -> &Self::Data {
            &self.data
        }

        fn code_boc(&self) -> Cow<'static, [u8]> {
            Cow::Owned(self.code_boc.clone())
        }

        fn workchain(&self) -> i8 {
            self.workchain
        }
    }

    #[test]
    fn contract_blueprint_builds_state_init_from_code_boc_and_typed_data() {
        let mut code_builder = Builder::new();
        code_builder.store_uint::<u8>(0xaa).unwrap();
        let code = code_builder.build().unwrap();
        let blueprint = TestBlueprint {
            code_boc: crate::tvm::serialize_boc(&code, false).unwrap(),
            data: BlueprintData { value: 0x1234_5678 },
            workchain: -1,
        };

        let state_init = blueprint.state_init().unwrap();

        assert_eq!(state_init.code.unwrap().hash(), code.hash());
        let data = state_init.data.unwrap();
        let mut slice = crate::tvm::Slice::new(data);
        assert_eq!(slice.load_uint::<u32>().unwrap(), 0x1234_5678);
        assert!(slice.is_empty());
    }

    #[tokio::test]
    async fn contract_blueprint_address_and_bind_use_derived_state_init() {
        let code = Builder::new().build().unwrap();
        let blueprint = TestBlueprint {
            code_boc: crate::tvm::serialize_boc(&code, false).unwrap(),
            data: BlueprintData { value: 7 },
            workchain: -1,
        };
        let expected = address_from_state_init(-1, &blueprint.state_init().unwrap()).unwrap();
        let latest = block(51);
        let mut provider = mock_provider(expected.clone(), latest, crate::tlb::Account::None);

        let contract = blueprint.bind(&mut provider).unwrap();

        assert_eq!(blueprint.address().unwrap(), expected);
        assert_eq!(contract.address(), &expected);
    }

    struct SeqnoContract<'a, P: ContractProvider + ?Sized> {
        inner: Contract<'a, P>,
    }

    impl<'a, P: ContractProvider + ?Sized> SeqnoContract<'a, P> {
        fn new(provider: &'a mut P, address: Address) -> Self {
            Self {
                inner: Contract::new(provider, address),
            }
        }

        async fn seqno(&mut self) -> Result<Vec<TvmStackEntry>, ContractError<P::Error>> {
            self.inner
                .run_get_method_by_name_typed_latest("seqno", TvmStack::empty())
                .await
        }
    }

    #[tokio::test]
    async fn address_bound_contract_can_be_embedded_in_typed_client() {
        let address = Address::new(0, [1; 32]);
        let latest = block(50);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let mut seqno = SeqnoContract::new(&mut provider, address);

        assert_eq!(seqno.seqno().await.unwrap(), vec![TvmStackEntry::int(7)]);
    }

    #[test]
    fn run_method_result_decodes_stack_losslessly() {
        let stack = TvmStack::new(vec![TvmStackEntry::int(10)]);
        let result = RunMethodResult {
            mode: (),
            id: block(1),
            shardblk: block(1),
            shard_proof: None,
            proof: None,
            state_proof: None,
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: Some(stack.to_boc().unwrap()),
        };

        assert_eq!(result.decode_result_stack().unwrap(), Some(stack.clone()));
        assert_eq!(
            result.result_stack_lossless(),
            DecodedRunMethodResult::Decoded(stack)
        );

        let result = RunMethodResult {
            result: Some(vec![0xff]),
            ..result
        };
        assert!(matches!(
            result.result_stack_lossless(),
            DecodedRunMethodResult::Undecodable { raw, .. } if raw == vec![0xff]
        ));
    }

    #[cfg(feature = "network-config")]
    #[ignore = "requires TON_GLOBAL_CONFIG_JSON, TON_KNOWN_CONTRACT_ADDRESS, and live network access"]
    #[tokio::test]
    async fn live_known_contract_seqno_placeholder() -> anyhow::Result<()> {
        use std::str::FromStr;

        let config_json = std::env::var("TON_GLOBAL_CONFIG_JSON")?;
        let address = std::env::var("TON_KNOWN_CONTRACT_ADDRESS")?;
        let config = crate::network_config::ConfigGlobal::from_str(&config_json)?;
        let mut client = LiteClient::connect_config(&config, 0).await?;
        let address = Address::from_str(&address)?;
        let mut contract = Contract::new(&mut client, address);
        let result = contract
            .run_get_method_by_name_latest("seqno", TvmStack::empty())
            .await?;
        assert!(result.exit_code >= 0);
        Ok(())
    }
}
