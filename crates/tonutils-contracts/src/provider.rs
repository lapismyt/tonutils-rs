use async_trait::async_trait;
use std::borrow::Cow;

use tonutils_tl::{
    BlockIdExt,
    common::{AccountId, Int256},
    response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
};
use tonutils_tlb::{StateInit, TlbSerialize};
use tonutils_tvm::{Address, TvmStack, TvmStackEntry, deserialize_boc};

use super::{FromTvmStack, ToTvmStack, TvmStackConversionError};

/// Errors returned by low-level contract execution helpers.
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
    #[error("failed to convert TVM stack value: {0}")]
    StackConversion(#[source] TvmStackConversionError),
}

/// Errors returned while building a contract address from fixed code and typed
/// TL-B data.
#[derive(Debug, thiserror::Error)]
pub enum ContractBuildError {
    #[error("failed to decode contract code BoC: {0}")]
    InvalidCodeBoc(#[source] anyhow::Error),
    #[error("failed to serialize contract data as TL-B: {0}")]
    DataSerialization(#[source] tonutils_tlb::TlbError),
    #[error("failed to serialize contract StateInit: {0}")]
    StateInitSerialization(#[source] anyhow::Error),
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
    fn stack_conversion(error: TvmStackConversionError) -> Self {
        Self::StackConversion(error)
    }
}

/// Raw LiteAPI operations required by the generic contract wrapper.
#[async_trait]
pub trait ContractProvider: Send {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error>;
    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error>;
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

/// A smart contract bound to an address and a raw LiteAPI provider.
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

    pub async fn run_get_method(
        &mut self,
        block: BlockIdExt,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        self.provider
            .run_get_method(
                crate::RUN_METHOD_MODE_RETURN_RESULT,
                block,
                self.address.clone(),
                method_id,
                stack,
            )
            .await
    }

    pub async fn run_get_method_by_name(
        &mut self,
        block: BlockIdExt,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<RunMethodResult, P::Error> {
        self.run_get_method(block, crate::method_name_to_id(method_name), stack)
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
        self.run_get_method_latest(crate::method_name_to_id(method_name), stack)
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
        self.run_get_method_typed(block, crate::method_name_to_id(method_name), stack)
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
        self.run_get_method_typed_latest(crate::method_name_to_id(method_name), stack)
            .await
    }

    pub async fn run_get_method_as<A, R>(
        &mut self,
        block: BlockIdExt,
        method_id: u64,
        args: A,
    ) -> Result<R, ContractError<P::Error>>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        let stack = args
            .to_tvm_stack()
            .map_err(ContractError::stack_conversion)?;
        let entries = self.run_get_method_typed(block, method_id, stack).await?;
        R::from_tvm_stack(TvmStack::new(entries)).map_err(ContractError::stack_conversion)
    }

    pub async fn run_get_method_by_name_as<A, R>(
        &mut self,
        block: BlockIdExt,
        method_name: &str,
        args: A,
    ) -> Result<R, ContractError<P::Error>>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        self.run_get_method_as(block, crate::method_name_to_id(method_name), args)
            .await
    }

    pub async fn run_get_method_latest_as<A, R>(
        &mut self,
        method_id: u64,
        args: A,
    ) -> Result<R, ContractError<P::Error>>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.run_get_method_as(block, method_id, args).await
    }

    pub async fn run_get_method_by_name_latest_as<A, R>(
        &mut self,
        method_name: &str,
        args: A,
    ) -> Result<R, ContractError<P::Error>>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        self.run_get_method_latest_as(crate::method_name_to_id(method_name), args)
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

/// A contract definition with fixed code BoC bytes and typed TL-B data.
pub trait ContractBlueprint {
    type Data: TlbSerialize;
    fn data(&self) -> &Self::Data;
    fn code_boc(&self) -> Cow<'static, [u8]>;
    fn workchain(&self) -> i8 {
        0
    }
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
    fn address(&self) -> Result<Address, ContractBuildError> {
        address_from_state_init(self.workchain(), &self.state_init()?)
            .map_err(ContractBuildError::StateInitSerialization)
    }
    fn bind<'a, P: ContractProvider + ?Sized>(
        &self,
        provider: &'a mut P,
    ) -> Result<Contract<'a, P>, ContractBuildError> {
        Ok(Contract::new(provider, self.address()?))
    }
}

pub fn address_from_state_init(
    workchain: i8,
    state_init: &StateInit,
) -> Result<Address, anyhow::Error> {
    Ok(Address::new(workchain, state_init.to_cell()?.hash()))
}

pub(super) fn decode_success_stack<E>(
    result: RunMethodResult,
) -> Result<Vec<TvmStackEntry>, ContractError<E>>
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
