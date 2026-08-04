use async_trait::async_trait;
use tonutils_contracts::ContractProvider;
use tonutils_tl::{
    BlockIdExt,
    common::{AccountId, Int256},
    response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
};
use tonutils_tvm::{Address, TvmStack};

use super::{
    balancer::{BalancerError, LiteBalancer},
    client::LiteClient,
    types::LiteError,
};

#[async_trait]
impl ContractProvider for LiteClient {
    type Error = LiteError;
    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        Self::get_masterchain_info(self).await
    }
    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        Self::get_account_state(self, block, account).await
    }
    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        Self::run_get_method(self, mode, block, account, method_id, stack).await
    }
    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
        Self::send_message(self, body).await
    }
    async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        Self::get_transactions(self, count, account, lt, hash).await
    }
}

#[async_trait]
impl ContractProvider for LiteBalancer {
    type Error = BalancerError;
    async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
        Self::get_masterchain_info(self).await
    }
    async fn get_account_state(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState, Self::Error> {
        Self::get_account_state(self, block, account).await
    }
    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error> {
        Self::run_get_method(self, mode, block, account, method_id, stack).await
    }
    async fn send_external_message_boc(&mut self, body: Vec<u8>) -> Result<u32, Self::Error> {
        Self::send_message(self, body).await
    }
    async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList, Self::Error> {
        Self::get_transactions(self, count, account, lt, hash).await
    }
}
