//! High-level smart-contract helpers built on LiteAPI calls.

use async_trait::async_trait;

use crate::liteclient::{balancer::LiteBalancer, client::LiteClient};
use crate::tl::{
    BlockIdExt,
    common::AccountId,
    response::{AccountState, MasterchainInfo, RunMethodResult},
};
use crate::tvm::{Address, TvmStack};

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

    async fn run_get_method(
        &mut self,
        mode: u32,
        block: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult, Self::Error>;
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
    use crate::tvm::TvmStackEntry;

    #[derive(Debug, thiserror::Error)]
    #[error("mock provider error")]
    struct MockError;

    struct MockProvider {
        latest: BlockIdExt,
        account: Address,
        state_calls: usize,
        method_calls: Vec<u64>,
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
            Ok(AccountState {
                id: self.latest.clone(),
                shardblk: self.latest.clone(),
                shard_proof: vec![1, 2],
                proof: vec![3, 4],
                state: vec![5, 6],
            })
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
                exit_code: 0,
                result: Some(TvmStack::new(vec![TvmStackEntry::int(7)]).to_boc().unwrap()),
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

    #[tokio::test]
    async fn contract_uses_provider_for_latest_state() {
        let address = Address::new(0, [9; 32]);
        let latest = block(42);
        let mut provider = MockProvider {
            latest,
            account: address.clone(),
            state_calls: 0,
            method_calls: Vec::new(),
        };

        let mut contract = Contract::new(&mut provider, address);
        let state = contract.get_state_latest().await.unwrap();

        assert_eq!(state.state, vec![5, 6]);
        assert_eq!(provider.state_calls, 1);
    }

    #[tokio::test]
    async fn contract_routes_method_name_to_id() {
        let address = Address::new(0, [8; 32]);
        let latest = block(43);
        let mut provider = MockProvider {
            latest,
            account: address.clone(),
            state_calls: 0,
            method_calls: Vec::new(),
        };

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
