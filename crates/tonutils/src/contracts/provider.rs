use std::borrow::Cow;

use crate::{
    contracts::stack::{FromTvmStack, ToTvmStack, TvmStackConversionError},
    provider::{
        AccountState, BlockId, MethodRef, Provider, ProviderError, ProviderResult, RunMethodResult,
        SendMessageResult, TransactionList,
    },
    tl::TlInt256 as Int256,
    tlb::{StateInit, TlbError, TlbSerialize, tlb_to_tvm_error},
    tvm::{Address, TvmError, TvmResult, TvmStack, TvmStackEntry, deserialize_boc},
};

/// Errors returned by low-level contract execution helpers.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("contract provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("get-method exited with code {exit_code}")]
    NonZeroExitCode { exit_code: i32 },
    #[error("failed to decode contract data: {0}")]
    Decode(#[source] TvmError),
    #[error("failed to convert TVM stack value: {0}")]
    StackConversion(#[source] TvmStackConversionError),
    #[error(transparent)]
    Build(#[from] ContractBuildError),
    #[error("contract data error: {0}")]
    Data(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl ContractError {
    fn stack_conversion(error: TvmStackConversionError) -> Self {
        Self::StackConversion(error)
    }
}

/// Errors returned while building a contract address from fixed code and typed
/// TL-B data.
#[derive(Debug, thiserror::Error)]
pub enum ContractBuildError {
    #[error("failed to decode contract code BoC: {0}")]
    InvalidCodeBoc(#[source] TvmError),
    #[error("failed to resolve embedded contract code: {0}")]
    Code(#[from] super::codes::ContractCodeError),
    #[error("failed to serialize contract data as TL-B: {0}")]
    DataSerialization(#[source] TlbError),
    #[error("failed to decode contract data from TL-B: {0}")]
    DataDecode(#[source] TlbError),
    #[error("failed to serialize contract StateInit: {0}")]
    StateInitSerialization(#[source] TvmError),
    #[error("contract data is missing required field {0}")]
    MissingField(&'static str),
}

/// A smart contract bound to an address and a provider.
pub struct Contract<'a, P: Provider + ?Sized> {
    provider: &'a mut P,
    address: Address,
}

impl<'a, P: Provider + ?Sized> Contract<'a, P> {
    pub fn new(provider: &'a mut P, address: Address) -> Self {
        Self { provider, address }
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Reads the account state at `block`, or at the latest block when `None`.
    pub async fn get_state(&mut self, block: Option<BlockId>) -> ProviderResult<AccountState> {
        self.provider
            .get_account_state(self.address.clone(), block)
            .await
    }

    /// Reads the account state at the latest masterchain block.
    pub async fn get_state_latest(&mut self) -> ProviderResult<AccountState> {
        self.get_state(None).await
    }

    /// Runs a get-method at `block`, or at the latest block when `None`.
    pub async fn run_get_method(
        &mut self,
        block: Option<BlockId>,
        method: MethodRef<'_>,
        stack: TvmStack,
    ) -> ProviderResult<RunMethodResult> {
        self.provider
            .run_get_method(self.address.clone(), method, stack, block)
            .await
    }

    pub async fn run_get_method_by_name(
        &mut self,
        block: Option<BlockId>,
        method_name: &str,
        stack: TvmStack,
    ) -> ProviderResult<RunMethodResult> {
        self.run_get_method(block, MethodRef::Name(method_name), stack)
            .await
    }

    pub async fn run_get_method_latest(
        &mut self,
        method: MethodRef<'_>,
        stack: TvmStack,
    ) -> ProviderResult<RunMethodResult> {
        self.run_get_method(None, method, stack).await
    }

    pub async fn run_get_method_by_name_latest(
        &mut self,
        method_name: &str,
        stack: TvmStack,
    ) -> ProviderResult<RunMethodResult> {
        self.run_get_method_latest(MethodRef::Name(method_name), stack)
            .await
    }

    pub async fn run_get_method_typed(
        &mut self,
        block: Option<BlockId>,
        method: MethodRef<'_>,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError> {
        let result = self.run_get_method(block, method, stack).await?;
        decode_success_stack(&result)
    }

    pub async fn run_get_method_by_name_typed(
        &mut self,
        block: Option<BlockId>,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError> {
        self.run_get_method_typed(block, MethodRef::Name(method_name), stack)
            .await
    }

    pub async fn run_get_method_typed_latest(
        &mut self,
        method: MethodRef<'_>,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError> {
        self.run_get_method_typed(None, method, stack).await
    }

    pub async fn run_get_method_by_name_typed_latest(
        &mut self,
        method_name: &str,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>, ContractError> {
        self.run_get_method_typed_latest(MethodRef::Name(method_name), stack)
            .await
    }

    pub async fn run_get_method_as<A, R>(
        &mut self,
        block: Option<BlockId>,
        method: MethodRef<'_>,
        args: A,
    ) -> Result<R, ContractError>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        let stack = args
            .to_tvm_stack()
            .map_err(ContractError::stack_conversion)?;
        let entries = self.run_get_method_typed(block, method, stack).await?;
        R::from_tvm_stack(TvmStack::new(entries)).map_err(ContractError::stack_conversion)
    }

    pub async fn run_get_method_by_name_as<A, R>(
        &mut self,
        block: Option<BlockId>,
        method_name: &str,
        args: A,
    ) -> Result<R, ContractError>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        self.run_get_method_as(block, MethodRef::Name(method_name), args)
            .await
    }

    pub async fn run_get_method_latest_as<A, R>(
        &mut self,
        method: MethodRef<'_>,
        args: A,
    ) -> Result<R, ContractError>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        self.run_get_method_as(None, method, args).await
    }

    pub async fn run_get_method_by_name_latest_as<A, R>(
        &mut self,
        method_name: &str,
        args: A,
    ) -> Result<R, ContractError>
    where
        A: ToTvmStack,
        R: FromTvmStack,
    {
        self.run_get_method_latest_as(MethodRef::Name(method_name), args)
            .await
    }

    pub async fn send_external_message_boc(
        &mut self,
        body: Vec<u8>,
    ) -> ProviderResult<SendMessageResult> {
        self.provider.send_external_message_boc(body).await
    }

    pub async fn get_transactions(
        &mut self,
        count: u32,
        lt: u64,
        hash: Int256,
    ) -> ProviderResult<TransactionList> {
        self.provider
            .get_transactions(count, self.address.clone(), lt, hash)
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
    fn bind<'a, P: Provider + ?Sized>(
        &self,
        provider: &'a mut P,
    ) -> Result<Contract<'a, P>, ContractBuildError> {
        Ok(Contract::new(provider, self.address()?))
    }
}

pub fn address_from_state_init(workchain: i8, state_init: &StateInit) -> TvmResult<Address> {
    let cell = state_init.to_cell().map_err(tlb_to_tvm_error)?;
    Ok(Address::new(workchain, cell.hash()))
}

pub(super) fn decode_success_stack(
    result: &RunMethodResult,
) -> Result<Vec<TvmStackEntry>, ContractError> {
    if result.exit_code != 0 {
        return Err(ContractError::NonZeroExitCode {
            exit_code: result.exit_code,
        });
    }
    Ok(result.stack.entries().to_vec())
}
