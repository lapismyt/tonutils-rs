use super::*;

use async_trait::async_trait;

use crate::abi::{
    AbiCodecError, AbiFunction, AbiFunctionKind, AbiSelector, AbiValue, decode_get_method_outputs,
    encode_get_method_inputs, encode_message_body,
};
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

use super::{FromTvmStack, ToTvmStack, TvmStackConversionError};

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
    #[error("failed to convert TVM stack value: {0}")]
    StackConversion(#[source] TvmStackConversionError),
    #[error("ABI contract call failed: {0}")]
    Abi(#[source] AbiCodecError),
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

    fn abi(error: AbiCodecError) -> Self {
        Self::Abi(error)
    }

    fn stack_conversion(error: TvmStackConversionError) -> Self {
        Self::StackConversion(error)
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
        self.run_get_method_as(block, crate::utils::method_name_to_id(method_name), args)
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
        self.run_get_method_latest_as(crate::utils::method_name_to_id(method_name), args)
            .await
    }

    pub async fn run_abi_get_method(
        &mut self,
        block: BlockIdExt,
        function: &AbiFunction,
        values: &[AbiValue],
    ) -> Result<Vec<AbiValue>, ContractError<P::Error>> {
        let method_id = abi_get_method_id(function).map_err(ContractError::abi)?;
        let entries = encode_get_method_inputs(function, values).map_err(ContractError::abi)?;
        let stack = TvmStack::new(entries);
        let result = self.run_get_method_typed(block, method_id, stack).await?;
        decode_get_method_outputs(function, &result).map_err(ContractError::abi)
    }

    pub async fn run_abi_get_method_latest(
        &mut self,
        function: &AbiFunction,
        values: &[AbiValue],
    ) -> Result<Vec<AbiValue>, ContractError<P::Error>> {
        let block = self
            .provider
            .get_masterchain_info()
            .await
            .map_err(ContractError::provider)?
            .last;
        self.run_abi_get_method(block, function, values).await
    }

    /// Builds the body cell for an ABI internal or external message function.
    ///
    /// This returns only the message body cell. It does not construct, sign, or
    /// serialize a full external message BoC.
    pub fn build_abi_message_body(
        &self,
        function: &AbiFunction,
        values: &[AbiValue],
    ) -> Result<Arc<Cell>, ContractError<P::Error>> {
        encode_message_body(function, values).map_err(ContractError::abi)
    }

    /// Builds the body cell for an ABI external inbound message function.
    ///
    /// This returns only the message body cell. It does not construct, sign, or
    /// serialize a full external message BoC.
    pub fn build_abi_external_message_body(
        &self,
        function: &AbiFunction,
        values: &[AbiValue],
    ) -> Result<Arc<Cell>, ContractError<P::Error>> {
        ensure_abi_message_kind(function, AbiFunctionKind::ExternalMessage)
            .map_err(ContractError::abi)?;
        self.build_abi_message_body(function, values)
    }

    /// Builds the body cell for an ABI internal inbound message function.
    pub fn build_abi_internal_message_body(
        &self,
        function: &AbiFunction,
        values: &[AbiValue],
    ) -> Result<Arc<Cell>, ContractError<P::Error>> {
        ensure_abi_message_kind(function, AbiFunctionKind::InternalMessage)
            .map_err(ContractError::abi)?;
        self.build_abi_message_body(function, values)
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

fn abi_get_method_id(function: &AbiFunction) -> Result<u64, AbiCodecError> {
    match (function.kind, function.selector) {
        (AbiFunctionKind::GetMethod, AbiSelector::MethodId(method_id)) => Ok(method_id),
        (AbiFunctionKind::GetMethod, AbiSelector::None) => {
            Ok(crate::utils::method_name_to_id(&function.name))
        }
        _ => Err(AbiCodecError::InvalidGetMethodSelector {
            kind: function.kind,
            selector: function.selector,
        }),
    }
}

fn ensure_abi_message_kind(
    function: &AbiFunction,
    expected: AbiFunctionKind,
) -> Result<(), AbiCodecError> {
    if function.kind != expected {
        return Err(AbiCodecError::InvalidMessageSelector {
            kind: function.kind,
            selector: function.selector,
        });
    }
    Ok(())
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

pub(super) fn active_state_init<E>(
    decoded: &DecodedAccountState,
) -> Result<StateInit, ContractError<E>>
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

pub(super) fn active_balance<E>(
    decoded: &DecodedAccountState,
) -> Result<CurrencyCollection, ContractError<E>>
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
