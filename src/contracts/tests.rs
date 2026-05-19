use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{
        AbiCodecError, AbiFunction, AbiFunctionKind, AbiParameter, AbiSelector, AbiType, AbiValue,
    };
    use crate::liteclient::boc::{DecodedAccountState, SimpleAccount};
    #[cfg(feature = "network-config")]
    use crate::liteclient::client::LiteClient;
    use crate::tl::response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList};
    use crate::tl::{AccountId, BlockIdExt, Int256};
    use crate::tlb::{
        AccountStorage, CurrencyCollection, Grams, MsgAddressInt, StateInit, StorageExtraInfo,
        StorageInfo, StorageUsed, TlbSerialize,
    };
    use crate::tvm::{Address, Builder, TvmStack, TvmStackEntry};
    use async_trait::async_trait;
    use num_bigint::{BigInt, BigUint};
    use std::borrow::Cow;
    use std::sync::Arc;

    #[derive(Debug, thiserror::Error)]
    #[error("mock provider error")]
    struct MockError;

    struct MockProvider {
        latest: BlockIdExt,
        account: Address,
        raw_state: AccountState,
        state_calls: usize,
        method_calls: Vec<u64>,
        method_stacks: Vec<TvmStack>,
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
            stack: TvmStack,
        ) -> Result<RunMethodResult, Self::Error> {
            assert_eq!(block, self.latest);
            assert_eq!(account, self.account);
            self.method_calls.push(method_id);
            self.method_stacks.push(stack);
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
            method_stacks: Vec::new(),
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
    async fn contract_forwards_non_empty_get_method_stack() {
        let address = Address::new(0, [8; 32]);
        let latest = block(43);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let stack = TvmStack::new(vec![TvmStackEntry::int(1), TvmStackEntry::Null]);

        let mut contract = Contract::new(&mut provider, address);
        contract
            .run_get_method_latest(85143, stack.clone())
            .await
            .unwrap();

        assert_eq!(provider.method_calls, vec![85143]);
        assert_eq!(provider.method_stacks, vec![stack]);
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

    #[test]
    fn stack_conversion_scalars_and_options_roundtrip() {
        assert_eq!(true.to_tvm_stack_entry().unwrap(), TvmStackEntry::int(-1));
        assert_eq!(false.to_tvm_stack_entry().unwrap(), TvmStackEntry::int(0));
        assert!(bool::from_tvm_stack(TvmStack::new(vec![TvmStackEntry::int(-1)])).unwrap());
        assert!(!bool::from_tvm_stack(TvmStack::new(vec![TvmStackEntry::int(0)])).unwrap());
        assert!(matches!(
            bool::from_tvm_stack_entry(TvmStackEntry::int(1)).unwrap_err(),
            TvmStackConversionError::InvalidBool { .. }
        ));

        let some = Some(5u32).to_tvm_stack().unwrap();
        assert_eq!(some.entries(), &[TvmStackEntry::int(5)]);
        assert_eq!(Option::<u32>::from_tvm_stack(some).unwrap(), Some(5));

        let none = Option::<u32>::None.to_tvm_stack().unwrap();
        assert_eq!(none.entries(), &[TvmStackEntry::Null]);
        assert_eq!(Option::<u32>::from_tvm_stack(none).unwrap(), None);

        assert_eq!(
            BigInt::from_tvm_stack(TvmStack::new(vec![TvmStackEntry::int(-9)])).unwrap(),
            BigInt::from(-9)
        );
        assert_eq!(
            BigUint::from_tvm_stack(TvmStack::new(vec![TvmStackEntry::int(9)])).unwrap(),
            BigUint::from(9u8)
        );
    }

    #[test]
    fn stack_conversion_reports_arity_and_integer_range_errors() {
        assert!(matches!(
            <(u8, u8)>::from_tvm_stack(TvmStack::new(vec![TvmStackEntry::int(1)])).unwrap_err(),
            TvmStackConversionError::StackArityMismatch {
                expected: 2,
                actual: 1
            }
        ));
        assert!(matches!(
            u8::from_tvm_stack_entry(TvmStackEntry::int(256)).unwrap_err(),
            TvmStackConversionError::IntegerOutOfRange { target: "u8", .. }
        ));
        assert!(matches!(
            u8::from_tvm_stack_entry(TvmStackEntry::int(-1)).unwrap_err(),
            TvmStackConversionError::IntegerOutOfRange { target: "u8", .. }
        ));
    }

    #[test]
    fn stack_conversion_addresses_cells_and_tuples_roundtrip() {
        let address = Address::new(0, [0x44; 32]);
        let address_entry = address.clone().to_tvm_stack_entry().unwrap();
        assert_eq!(
            Address::from_tvm_stack_entry(address_entry).unwrap(),
            address
        );

        let cell = Builder::new().build().unwrap();
        let cell_entry = cell.clone().to_tvm_stack_entry().unwrap();
        assert_eq!(
            Arc::<crate::tvm::Cell>::from_tvm_stack_entry(cell_entry)
                .unwrap()
                .hash(),
            cell.hash()
        );

        let tuple = (7u32, true, Option::<u8>::None).to_tvm_stack().unwrap();
        assert_eq!(
            <(u32, bool, Option<u8>)>::from_tvm_stack(tuple).unwrap(),
            (7, true, None)
        );

        let tuple_entry = (1u8, false).to_tvm_stack_entry().unwrap();
        assert_eq!(
            <(u8, bool)>::from_tvm_stack_entry(tuple_entry).unwrap(),
            (1, false)
        );
    }

    #[tokio::test]
    async fn typed_as_helpers_convert_arguments_and_results() {
        let address = Address::new(0, [0x33; 32]);
        let latest = block(57);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);

        let mut contract = Contract::new(&mut provider, address);
        let seqno: u32 = contract
            .run_get_method_by_name_latest_as("seqno", (true, 9u32))
            .await
            .unwrap();

        assert_eq!(seqno, 7);
        assert_eq!(
            provider.method_calls,
            vec![crate::utils::method_name_to_id("seqno")]
        );
        assert_eq!(
            provider.method_stacks,
            vec![TvmStack::new(vec![
                TvmStackEntry::int(-1),
                TvmStackEntry::int(9)
            ])]
        );
    }

    fn abi_parameter(name: &str, ty: AbiType) -> AbiParameter {
        AbiParameter {
            name: name.to_string(),
            ty,
            optional: false,
        }
    }

    fn abi_seqno_function(selector: AbiSelector) -> AbiFunction {
        AbiFunction {
            name: "seqno".to_string(),
            kind: AbiFunctionKind::GetMethod,
            selector,
            inputs: vec![abi_parameter("flag", AbiType::Bool)],
            outputs: vec![abi_parameter("seqno", AbiType::Uint { bits: 32 })],
        }
    }

    #[tokio::test]
    async fn abi_get_method_encodes_inputs_and_decodes_outputs() {
        let address = Address::new(0, [0x13; 32]);
        let latest = block(52);
        let mut provider =
            mock_provider(address.clone(), latest.clone(), crate::tlb::Account::None);
        let function = abi_seqno_function(AbiSelector::MethodId(0x1234));

        let mut contract = Contract::new(&mut provider, address);
        let values = contract
            .run_abi_get_method_latest(&function, &[AbiValue::Bool(true)])
            .await
            .unwrap();

        assert_eq!(values, vec![AbiValue::Uint(BigUint::from(7u8))]);
        assert_eq!(provider.method_calls, vec![0x1234]);
        assert_eq!(
            provider.method_stacks,
            vec![TvmStack::new(vec![TvmStackEntry::int(-1)])]
        );
    }

    #[tokio::test]
    async fn abi_get_method_uses_name_mapping_when_selector_is_absent() {
        let address = Address::new(0, [0x14; 32]);
        let latest = block(53);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let function = abi_seqno_function(AbiSelector::None);

        let mut contract = Contract::new(&mut provider, address);
        let _ = contract
            .run_abi_get_method_latest(&function, &[AbiValue::Bool(false)])
            .await
            .unwrap();

        assert_eq!(
            provider.method_calls,
            vec![crate::utils::method_name_to_id("seqno")]
        );
    }

    #[tokio::test]
    async fn abi_get_method_rejects_message_functions_before_provider_call() {
        let address = Address::new(0, [0x15; 32]);
        let latest = block(54);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let mut function = abi_seqno_function(AbiSelector::Opcode(1));
        function.kind = AbiFunctionKind::InternalMessage;

        let mut contract = Contract::new(&mut provider, address);
        assert!(matches!(
            contract.run_abi_get_method_latest(&function, &[]).await,
            Err(ContractError::Abi(
                AbiCodecError::InvalidGetMethodSelector {
                    kind: AbiFunctionKind::InternalMessage,
                    selector: AbiSelector::Opcode(1),
                },
            ))
        ));
        assert!(provider.method_calls.is_empty());
    }

    #[tokio::test]
    async fn abi_message_body_helpers_build_internal_and_external_cells() {
        let address = Address::new(0, [0x16; 32]);
        let latest = block(55);
        let mut provider = mock_provider(address.clone(), latest, crate::tlb::Account::None);
        let contract = Contract::new(&mut provider, address);
        let function = AbiFunction {
            name: "transfer".to_string(),
            kind: AbiFunctionKind::ExternalMessage,
            selector: AbiSelector::Opcode(0x1122_3344),
            inputs: vec![abi_parameter("amount", AbiType::Uint { bits: 32 })],
            outputs: Vec::new(),
        };

        let body = contract
            .build_abi_external_message_body(&function, &[AbiValue::Uint(BigUint::from(9u8))])
            .unwrap();
        let mut slice = crate::tvm::Slice::new(body);
        assert_eq!(slice.load_u32().unwrap(), 0x1122_3344);
        assert_eq!(slice.load_uint::<u32>().unwrap(), 9);
        assert!(slice.is_empty());

        let mut internal = function.clone();
        internal.kind = AbiFunctionKind::InternalMessage;
        assert!(
            contract
                .build_abi_internal_message_body(&internal, &[AbiValue::Uint(BigUint::from(9u8))])
                .is_ok()
        );
        assert!(matches!(
            contract.build_abi_external_message_body(&internal, &[]),
            Err(ContractError::Abi(AbiCodecError::InvalidMessageSelector {
                kind: AbiFunctionKind::InternalMessage,
                selector: AbiSelector::Opcode(0x1122_3344),
            }))
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

    #[cfg(feature = "network-config")]
    #[ignore = "requires TON_GLOBAL_CONFIG_JSON, TON_STACK_TEST_CONTRACT_ADDRESS, TON_STACK_TEST_JSON, and live network access"]
    #[tokio::test]
    async fn live_non_empty_stack_run_get_method_smoke() -> anyhow::Result<()> {
        use crate::contracts::RunMethodResultExt;
        use crate::tvm::TvmStackEntry;
        use anyhow::Context as _;
        use serde_json::Value;
        use std::str::FromStr;

        fn parse_entry(value: &Value) -> anyhow::Result<TvmStackEntry> {
            let object = value
                .as_object()
                .context("TON_STACK_TEST_JSON entry must be an object")?;
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .context("TON_STACK_TEST_JSON entry must include string field \"type\"")?;
            match kind {
                "null" => Ok(TvmStackEntry::Null),
                "int" => {
                    let value = object
                        .get("value")
                        .and_then(Value::as_str)
                        .context("int entry must include string field \"value\"")?;
                    Ok(TvmStackEntry::Int(
                        BigInt::parse_bytes(value.as_bytes(), 10)
                            .context("invalid decimal int entry")?,
                    ))
                }
                "cell" | "slice" => {
                    let boc = object
                        .get("boc")
                        .and_then(Value::as_str)
                        .context("cell/slice entry must include string field \"boc\"")?;
                    let cell = crate::tvm::deserialize_boc(&hex::decode(boc.trim())?)?;
                    if kind == "cell" {
                        Ok(TvmStackEntry::Cell(cell))
                    } else {
                        Ok(TvmStackEntry::Slice(cell))
                    }
                }
                "tuple" | "list" => {
                    let entries = object
                        .get("entries")
                        .and_then(Value::as_array)
                        .context("tuple/list entry must include array field \"entries\"")?
                        .iter()
                        .map(parse_entry)
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    if kind == "tuple" {
                        Ok(TvmStackEntry::Tuple(entries))
                    } else {
                        Ok(TvmStackEntry::List(entries))
                    }
                }
                "unsupported" => {
                    let raw = object
                        .get("raw")
                        .and_then(Value::as_str)
                        .context("unsupported entry must include string field \"raw\"")?;
                    Ok(TvmStackEntry::Unsupported(hex::decode(raw.trim())?))
                }
                _ => anyhow::bail!("unsupported stack entry type {kind}"),
            }
        }

        let config_json = std::env::var("TON_GLOBAL_CONFIG_JSON")?;
        let address = std::env::var("TON_STACK_TEST_CONTRACT_ADDRESS")?;
        let method = std::env::var("TON_STACK_TEST_METHOD").unwrap_or_else(|_| "seqno".to_owned());
        let stack_json = std::env::var("TON_STACK_TEST_JSON")?;
        let stack_value: Value = serde_json::from_str(&stack_json)?;
        let stack_entries = stack_value
            .as_array()
            .context("TON_STACK_TEST_JSON root must be an array")?
            .iter()
            .map(parse_entry)
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(
            !stack_entries.is_empty(),
            "TON_STACK_TEST_JSON must describe a non-empty stack"
        );

        let config = crate::network_config::ConfigGlobal::from_str(&config_json)?;
        let mut client = LiteClient::connect_config(&config, 0).await?;
        let address = Address::from_str(&address)?;
        let address_raw = address.to_raw();
        let mut contract = Contract::new(&mut client, address);
        let params_stack = TvmStack::new(stack_entries);
        let params_boc = params_stack.to_boc()?;
        let params_root_hash = hex::encode(params_stack.to_cell()?.hash());
        let result = contract
            .run_get_method_by_name_latest(&method, params_stack)
            .await?;

        if result.exit_code != 0 {
            let accepted = std::env::var("TON_STACK_TEST_ACCEPT_EXIT_CODE")
                .ok()
                .and_then(|value| value.parse::<i32>().ok());
            assert_eq!(accepted, Some(result.exit_code));
            return Ok(());
        }

        match result.result_stack_lossless() {
            DecodedRunMethodResult::Decoded(stack) => {
                let result_boc = stack.to_boc()?;
                let fixture = serde_json::json!({
                    "schema_revision": 1,
                    "evidence_kind": "captured_or_opt_in",
                    "source_sdk_or_tool": "tonutils-rs live ignored test",
                    "source_version_or_commit": env!("CARGO_PKG_VERSION"),
                    "network": std::env::var("TON_STACK_TEST_NETWORK").unwrap_or_else(|_| "live".to_owned()),
                    "endpoint": "TON_GLOBAL_CONFIG_JSON peer index 0",
                    "block_id": {
                        "workchain": result.id.workchain,
                        "shard": result.id.shard,
                        "seqno": result.id.seqno,
                        "root_hash": hex::encode(result.id.root_hash.0),
                        "file_hash": hex::encode(result.id.file_hash.0)
                    },
                    "account": address_raw,
                    "method": method,
                    "input_stack_json": stack_value,
                    "params_boc_hex": hex::encode(params_boc),
                    "params_root_hash": params_root_hash,
                    "exit_code": result.exit_code,
                    "result_boc_hex": hex::encode(result_boc),
                    "result_root_hash": hex::encode(stack.to_cell()?.hash()),
                    "decoded_result": stack_entries_value(stack.entries()),
                    "compat_reference": "compare params_boc_hex with tonutils-go when raw params BoC is available; compare decoded_result structurally with tonlib otherwise"
                });
                println!("{}", serde_json::to_string_pretty(&fixture)?);
            }
            DecodedRunMethodResult::Undecodable { raw, .. } => assert!(!raw.is_empty()),
            DecodedRunMethodResult::Missing => anyhow::bail!("live get-method returned no stack"),
        }
        Ok(())
    }

    fn stack_entries_value(entries: &[TvmStackEntry]) -> serde_json::Value {
        serde_json::Value::Array(entries.iter().map(stack_entry_value).collect())
    }

    fn stack_entry_value(entry: &TvmStackEntry) -> serde_json::Value {
        match entry {
            TvmStackEntry::Null => serde_json::json!({ "type": "null" }),
            TvmStackEntry::Int(value) => {
                serde_json::json!({ "type": "int", "value": value.to_str_radix(10) })
            }
            TvmStackEntry::Cell(cell) => serde_json::json!({
                "type": "cell",
                "boc": hex::encode(crate::tvm::serialize_boc(cell, false).unwrap())
            }),
            TvmStackEntry::Slice(cell) => serde_json::json!({
                "type": "slice",
                "boc": hex::encode(crate::tvm::serialize_boc(cell, false).unwrap())
            }),
            TvmStackEntry::Tuple(entries) => {
                serde_json::json!({ "type": "tuple", "entries": stack_entries_value(entries) })
            }
            TvmStackEntry::List(entries) => {
                serde_json::json!({ "type": "list", "entries": stack_entries_value(entries) })
            }
            TvmStackEntry::Unsupported(bytes) => {
                serde_json::json!({ "type": "unsupported", "raw": hex::encode(bytes) })
            }
        }
    }
}
