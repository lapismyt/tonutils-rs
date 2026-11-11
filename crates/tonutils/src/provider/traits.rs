use async_trait::async_trait;

use crate::{
    tl::TlInt256 as Int256,
    tvm::{Address, TvmStack},
};

use super::{
    error::ProviderResult,
    types::{
        AccountState, BlockId, MasterchainInfo, MethodRef, RunMethodResult, SendMessageResult,
        TransactionList,
    },
};

/// Read the latest masterchain block.
#[async_trait]
pub trait GetMasterchainInfoExt: Send {
    async fn get_masterchain_info(&mut self) -> ProviderResult<MasterchainInfo>;
}

/// Read an account state.
#[async_trait]
pub trait GetAccountStateExt: Send {
    /// Reads an account state at the latest block when `block` is `None`.
    ///
    /// Providers that cannot address an explicit block return
    /// [`super::ProviderError::UnsupportedBlock`].
    async fn get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockId>,
    ) -> ProviderResult<AccountState>;
}

/// Execute a TVM get-method.
#[async_trait]
pub trait RunGetMethodExt: Send {
    /// Executes a get-method at the latest block when `block` is `None`.
    ///
    /// Providers that cannot address an explicit block return
    /// [`super::ProviderError::UnsupportedBlock`].
    async fn run_get_method(
        &mut self,
        account: Address,
        method: MethodRef<'_>,
        stack: TvmStack,
        block: Option<BlockId>,
    ) -> ProviderResult<RunMethodResult>;
}

/// Submit a serialized external message.
#[async_trait]
pub trait SendExternalMessageExt: Send {
    async fn send_external_message_boc(
        &mut self,
        body: Vec<u8>,
    ) -> ProviderResult<SendMessageResult>;
}

/// Read account transaction history.
#[async_trait]
pub trait GetTransactionsExt: Send {
    async fn get_transactions(
        &mut self,
        count: u32,
        account: Address,
        lt: u64,
        hash: Int256,
    ) -> ProviderResult<TransactionList>;
}

/// Marker for providers that implement every core capability.
///
/// Individual capabilities remain separate traits so callers can require only
/// what they use.
pub trait Provider:
    GetMasterchainInfoExt
    + GetAccountStateExt
    + RunGetMethodExt
    + SendExternalMessageExt
    + GetTransactionsExt
    + Send
{
}

impl<T> Provider for T where
    T: GetMasterchainInfoExt
        + GetAccountStateExt
        + RunGetMethodExt
        + SendExternalMessageExt
        + GetTransactionsExt
        + Send
{
}
