use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;
    use crate::metadata::{Tep64KnownKey, Tep64Value, tep64_key_hash};
    use crate::tlb::{Anycast, MsgAddress, MsgAddressExt, MsgAddressInt, TlbSerialize};
    use crate::tvm::{Address, BitKey, Builder, Cell, HashmapE, Slice, TvmStack, TvmStackEntry};
    use num_bigint::{BigInt, BigUint};
    use std::sync::Arc;

    const ON_CHAIN_TAG: u8 = 0x00;
    const OFF_CHAIN_TAG: u8 = 0x01;
    const TEP64_DICT_KEY_BITS: usize = 256;

    fn empty() -> Arc<Cell> {
        Builder::new().build().unwrap()
    }

    fn address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn address_cell(address: Address) -> Arc<Cell> {
        MsgAddress::Int(MsgAddressInt::std(address))
            .to_cell()
            .unwrap()
    }

    fn none_address_cell() -> Arc<Cell> {
        MsgAddress::Ext(MsgAddressExt::None).to_cell().unwrap()
    }

    fn offchain_content(uri: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(OFF_CHAIN_TAG).unwrap();
        builder.store_snake_bytes(uri).unwrap();
        builder.build().unwrap()
    }

    fn value_cell(tag: u8, bytes: &[u8]) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(tag).unwrap();
        builder.store_snake_bytes(bytes).unwrap();
        builder.build().unwrap()
    }

    fn malformed_value_cell() -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_u8(ON_CHAIN_TAG).unwrap();
        builder.store_bit(true).unwrap();
        builder.build().unwrap()
    }

    fn key(name: &str) -> BitKey {
        BitKey::from_bits(tep64_key_hash(name).to_vec(), TEP64_DICT_KEY_BITS).unwrap()
    }

    fn onchain_content(entries: &[(&str, Arc<Cell>)]) -> Arc<Cell> {
        let mut dict = HashmapE::new(TEP64_DICT_KEY_BITS);
        for (name, value) in entries {
            dict.insert_bit_key(key(name), value.clone()).unwrap();
        }
        let mut builder = Builder::new();
        builder.store_u8(ON_CHAIN_TAG).unwrap();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_ref(value.clone())?;
                Ok(())
            })
            .unwrap();
        builder.build().unwrap()
    }

    fn collection_stack(content: Arc<Cell>) -> Vec<TvmStackEntry> {
        vec![
            TvmStackEntry::int(10),
            TvmStackEntry::Cell(content),
            TvmStackEntry::Slice(address_cell(address(0x11))),
        ]
    }

    fn item_stack(content: Arc<Cell>) -> Vec<TvmStackEntry> {
        vec![
            TvmStackEntry::int(-1),
            TvmStackEntry::int(7),
            TvmStackEntry::Slice(address_cell(address(0x22))),
            TvmStackEntry::Slice(address_cell(address(0x33))),
            TvmStackEntry::Cell(content),
        ]
    }

    #[test]
    fn decodes_valid_collection_data_stack() {
        let data = decode_nft_collection_data(collection_stack(offchain_content(
            b"https://example.test/collection.json",
        )))
        .unwrap();

        assert_eq!(data.next_item_index, BigUint::from(10u32));
        assert_eq!(data.owner_address, Some(address(0x11)));
        assert_eq!(
            data.metadata().uri.as_deref(),
            Some("https://example.test/collection.json")
        );
    }

    #[test]
    fn decodes_valid_item_data_stack() {
        let data = decode_nft_item_data(item_stack(empty())).unwrap();

        assert!(data.initialized);
        assert_eq!(data.index, BigUint::from(7u32));
        assert_eq!(data.collection_address, Some(address(0x22)));
        assert_eq!(data.owner_address, Some(address(0x33)));
    }

    #[test]
    fn rejects_wrong_stack_length_and_types() {
        assert!(matches!(
            decode_nft_collection_data(vec![]),
            Err(NftMetadataError::StackLength {
                method: "get_collection_data",
                actual: 0,
                expected: 3
            })
        ));

        let mut stack = item_stack(empty());
        stack[1] = TvmStackEntry::Cell(empty());
        assert!(matches!(
            decode_nft_item_data(stack),
            Err(NftMetadataError::StackType {
                method: "get_nft_data",
                index: 1,
                expected: "integer",
                actual: "cell"
            })
        ));
    }

    #[test]
    fn rejects_negative_indices() {
        let mut stack = collection_stack(offchain_content(b"https://example.test"));
        stack[0] = TvmStackEntry::int(-1);
        assert!(matches!(
            decode_nft_collection_data(stack),
            Err(NftMetadataError::InvalidInteger {
                method: "get_collection_data",
                field: "next_item_index",
                ..
            })
        ));

        let mut stack = item_stack(empty());
        stack[1] = TvmStackEntry::int(-1);
        assert!(matches!(
            decode_nft_item_data(stack),
            Err(NftMetadataError::InvalidInteger {
                method: "get_nft_data",
                field: "index",
                ..
            })
        ));
    }

    #[test]
    fn init_maps_zero_to_false_and_non_zero_to_true() {
        let mut stack = item_stack(empty());
        stack[0] = TvmStackEntry::int(0);
        assert!(!decode_nft_item_data(stack).unwrap().initialized);

        let mut stack = item_stack(empty());
        stack[0] = TvmStackEntry::int(42);
        assert!(decode_nft_item_data(stack).unwrap().initialized);
    }

    #[test]
    fn accepts_addr_none_as_none() {
        let mut stack = item_stack(empty());
        stack[2] = TvmStackEntry::Slice(none_address_cell());
        stack[3] = TvmStackEntry::Slice(none_address_cell());

        let data = decode_nft_item_data(stack).unwrap();

        assert_eq!(data.collection_address, None);
        assert_eq!(data.owner_address, None);
    }

    #[test]
    fn rejects_malformed_trailing_external_var_and_anycast_addresses() {
        let malformed = {
            let mut builder = Builder::new();
            builder.store_bit(true).unwrap();
            builder.build().unwrap()
        };
        let trailing = {
            let mut builder = Builder::new();
            builder
                .store_slice(&Slice::new(address_cell(address(0x44))))
                .unwrap();
            builder.store_bit(true).unwrap();
            builder.build().unwrap()
        };
        let external = MsgAddress::Ext(MsgAddressExt::Extern {
            data: vec![0x80],
            bit_len: 1,
        })
        .to_cell()
        .unwrap();
        let var = MsgAddress::Int(MsgAddressInt::Var {
            anycast: None,
            workchain_id: 0,
            address: vec![0x80],
            bit_len: 1,
        })
        .to_cell()
        .unwrap();
        let anycast = MsgAddress::Int(MsgAddressInt::Std {
            anycast: Some(Anycast {
                depth: 1,
                rewrite_pfx: vec![0x80],
            }),
            address: address(0x55),
        })
        .to_cell()
        .unwrap();

        for cell in [malformed, trailing, external, var, anycast] {
            let mut stack = item_stack(empty());
            stack[2] = TvmStackEntry::Slice(cell);
            assert!(matches!(
                decode_nft_item_data(stack),
                Err(NftMetadataError::MalformedAddress {
                    method: "get_nft_data",
                    field: "collection_address",
                    ..
                })
            ));
        }
    }

    #[test]
    fn maps_full_nft_metadata_fields_and_preserves_unknowns() {
        let content = onchain_content(&[
            ("name", value_cell(ON_CHAIN_TAG, b"Example NFT")),
            ("description", value_cell(ON_CHAIN_TAG, b"Test item")),
            (
                "image",
                value_cell(ON_CHAIN_TAG, b"https://example.test/image.png"),
            ),
            ("image_data", value_cell(ON_CHAIN_TAG, b"<svg/>")),
            ("render_type", value_cell(ON_CHAIN_TAG, b"game")),
            (
                "content_url",
                value_cell(ON_CHAIN_TAG, b"https://example.test/content"),
            ),
            (
                "video",
                value_cell(ON_CHAIN_TAG, b"https://example.test/video.mp4"),
            ),
            ("symbol", value_cell(ON_CHAIN_TAG, b"NFT")),
            ("custom", value_cell(ON_CHAIN_TAG, b"custom-value")),
        ]);

        let metadata = parse_nft_metadata_cell(content).unwrap();

        assert_eq!(metadata.name.as_deref(), Some("Example NFT"));
        assert_eq!(metadata.description.as_deref(), Some("Test item"));
        assert_eq!(
            metadata.image.as_deref(),
            Some("https://example.test/image.png")
        );
        assert_eq!(metadata.image_data.as_deref(), Some(&b"<svg/>"[..]));
        assert_eq!(metadata.render_type.as_deref(), Some("game"));
        assert_eq!(
            metadata.content_url.as_deref(),
            Some("https://example.test/content")
        );
        assert_eq!(
            metadata.video.as_deref(),
            Some("https://example.test/video.mp4")
        );
        assert_eq!(metadata.unknown_fields.len(), 2);
        assert!(
            metadata
                .unknown_fields
                .iter()
                .any(|field| field.key_hash == tep64_key_hash("symbol"))
        );
        let custom = metadata
            .unknown_fields
            .iter()
            .find(|field| field.key_hash == tep64_key_hash("custom"))
            .unwrap();
        assert_eq!(custom.value, Tep64Value::Snake(b"custom-value".to_vec()));
    }

    #[test]
    fn malformed_known_field_becomes_diagnostic() {
        let metadata =
            parse_nft_metadata_cell(onchain_content(&[("name", malformed_value_cell())])).unwrap();

        assert_eq!(metadata.name, None);
        assert_eq!(metadata.field_diagnostics.len(), 1);
        assert_eq!(
            metadata.field_diagnostics[0].known_key,
            Some(Tep64KnownKey::Name)
        );
        assert!(metadata.field_diagnostics[0].error.contains("byte-aligned"));
    }

    #[test]
    fn decodes_get_nft_content_stack() {
        let metadata = decode_nft_full_content_metadata(vec![TvmStackEntry::Cell(
            offchain_content(b"https://example.test/item.json"),
        )])
        .unwrap();

        assert_eq!(
            metadata.uri.as_deref(),
            Some("https://example.test/item.json")
        );
    }

    #[cfg(feature = "liteclient")]
    mod provider_tests {
        use super::*;
        use crate::contracts::{Contract, ContractError, ContractProvider};
        use crate::liteclient::boc::{DecodedAccountState, SimpleAccount};
        use crate::tl::{
            BlockIdExt, Int256,
            common::{AccountId, ZeroStateIdExt},
            response::{AccountState, MasterchainInfo, RunMethodResult, TransactionList},
        };
        use async_trait::async_trait;

        #[derive(Debug, thiserror::Error)]
        #[error("mock nft provider error")]
        struct MockProviderError;

        struct ExpectedCall {
            account: Address,
            method_id: u64,
            stack: TvmStack,
            result: TvmStack,
            exit_code: i32,
        }

        struct MockProvider {
            latest: BlockIdExt,
            calls: Vec<ExpectedCall>,
            seen_methods: Vec<u64>,
            fail_run_method: bool,
        }

        #[async_trait]
        impl ContractProvider for MockProvider {
            type Error = MockProviderError;

            async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo, Self::Error> {
                Ok(MasterchainInfo {
                    last: self.latest.clone(),
                    state_root_hash: Int256([1; 32]),
                    init: ZeroStateIdExt {
                        workchain: -1,
                        root_hash: Int256([2; 32]),
                        file_hash: Int256([3; 32]),
                    },
                })
            }

            async fn get_account_state(
                &mut self,
                _block: BlockIdExt,
                _account: AccountId,
            ) -> Result<AccountState, Self::Error> {
                unreachable!("nft metadata helper must not fetch account state")
            }

            async fn get_account_state_typed(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<DecodedAccountState, Self::Error> {
                unreachable!("nft metadata helper must not fetch account state")
            }

            async fn get_account_state_simple(
                &mut self,
                _block: BlockIdExt,
                _account: Address,
            ) -> Result<SimpleAccount, Self::Error> {
                unreachable!("nft metadata helper must not fetch account state")
            }

            async fn run_get_method(
                &mut self,
                mode: u32,
                block: BlockIdExt,
                account: Address,
                method_id: u64,
                stack: TvmStack,
            ) -> Result<RunMethodResult, Self::Error> {
                if self.fail_run_method {
                    return Err(MockProviderError);
                }
                assert_eq!(mode, 0);
                assert_eq!(block, self.latest);
                let expected = self.calls.remove(0);
                assert_eq!(account, expected.account);
                assert_eq!(method_id, expected.method_id);
                assert_eq!(stack, expected.stack);
                self.seen_methods.push(method_id);
                Ok(RunMethodResult {
                    mode: (),
                    id: self.latest.clone(),
                    shardblk: self.latest.clone(),
                    shard_proof: None,
                    proof: None,
                    state_proof: None,
                    init_c7: None,
                    lib_extras: None,
                    exit_code: expected.exit_code,
                    result: Some(expected.result.to_boc().unwrap()),
                })
            }

            async fn send_external_message_boc(
                &mut self,
                _body: Vec<u8>,
            ) -> Result<u32, Self::Error> {
                unreachable!("nft metadata helper must not send messages")
            }

            async fn get_transactions(
                &mut self,
                _count: u32,
                _account: AccountId,
                _lt: u64,
                _hash: Int256,
            ) -> Result<TransactionList, Self::Error> {
                unreachable!("nft metadata helper must not fetch transactions")
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

        fn mock_provider(calls: Vec<ExpectedCall>) -> MockProvider {
            MockProvider {
                latest: block(10),
                calls,
                seen_methods: Vec::new(),
                fail_run_method: false,
            }
        }

        fn call(
            account: Address,
            method: &str,
            stack: TvmStack,
            result: TvmStack,
            exit_code: i32,
        ) -> ExpectedCall {
            ExpectedCall {
                account,
                method_id: crate::utils::method_name_to_id(method),
                stack,
                result,
                exit_code,
            }
        }

        #[tokio::test]
        async fn collection_helper_uses_latest_block_empty_stack_and_method_id() {
            let account = address(0xaa);
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_collection_data",
                TvmStack::empty(),
                TvmStack::new(collection_stack(offchain_content(b"https://example.test"))),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let data = contract.nft_collection_data_latest().await.unwrap();

            assert_eq!(data.next_item_index, BigUint::from(10u32));
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_collection_data")]
            );
        }

        #[tokio::test]
        async fn item_helper_uses_latest_block_empty_stack_and_method_id() {
            let account = address(0xab);
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::new(item_stack(offchain_content(
                    b"https://example.test/item.json",
                ))),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let data = contract.nft_item_data_latest().await.unwrap();

            assert_eq!(data.index, BigUint::from(7u32));
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_nft_data")]
            );
        }

        #[tokio::test]
        async fn full_item_helper_passes_index_and_individual_content() {
            let collection = address(0xac);
            let individual_content = offchain_content(b"7.json");
            let item_data = NftItemData {
                initialized: true,
                index: BigUint::from(7u32),
                collection_address: Some(collection.clone()),
                owner_address: Some(address(0xad)),
                individual_content: individual_content.clone(),
            };
            let expected_stack = TvmStack::new(vec![
                TvmStackEntry::Int(BigInt::from(7u32)),
                TvmStackEntry::Cell(individual_content),
            ]);
            let mut provider = mock_provider(vec![call(
                collection.clone(),
                "get_nft_content",
                expected_stack,
                TvmStack::new(vec![TvmStackEntry::Cell(offchain_content(
                    b"https://example.test/item/7.json",
                ))]),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, collection);
            let metadata = contract
                .nft_full_item_metadata_latest(&item_data)
                .await
                .unwrap();

            assert_eq!(
                metadata.uri.as_deref(),
                Some("https://example.test/item/7.json")
            );
            assert_eq!(
                provider.seen_methods,
                vec![crate::utils::method_name_to_id("get_nft_content")]
            );
        }

        #[tokio::test]
        async fn standalone_item_metadata_parses_individual_content_directly() {
            let account = address(0xae);
            let mut stack = item_stack(offchain_content(b"https://example.test/standalone.json"));
            stack[2] = TvmStackEntry::Slice(none_address_cell());
            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::new(stack),
                0,
            )]);

            let mut contract = Contract::new(&mut provider, account);
            let metadata = contract.nft_item_metadata_latest().await.unwrap();

            assert_eq!(
                metadata.uri.as_deref(),
                Some("https://example.test/standalone.json")
            );
        }

        #[tokio::test]
        async fn provider_helper_propagates_provider_errors_and_exit_codes() {
            let account = address(0xaf);
            let mut provider = mock_provider(Vec::new());
            provider.fail_run_method = true;
            let mut contract = Contract::new(&mut provider, account.clone());
            assert!(matches!(
                contract.nft_item_data_latest().await,
                Err(ContractError::Provider(_))
            ));

            let mut provider = mock_provider(vec![call(
                account.clone(),
                "get_nft_data",
                TvmStack::empty(),
                TvmStack::empty(),
                13,
            )]);
            let mut contract = Contract::new(&mut provider, account);
            assert!(matches!(
                contract.nft_item_data_latest().await,
                Err(ContractError::NonZeroExitCode { exit_code: 13 })
            ));
        }
    }
}
