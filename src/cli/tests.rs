use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn fixture_mnemonic() -> TonMnemonic {
        TonMnemonic::from_phrase(
            "open price dish charge law skirt alien churn fire swap number brass outdoor diamond lesson april remain puzzle title elbow valley grant champion staff",
            None,
        )
        .unwrap()
    }

    fn transfer_args(version: WalletVersionArg, deploy: bool) -> WalletTransferArgs {
        WalletTransferArgs {
            version,
            to: "0:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            amount: 1_000,
            comment: None,
            mode: 3,
            timeout: 60,
            seqno: Some(7),
            wallet_id: None,
            workchain: 0,
            deploy,
            mnemonic_file: None,
            mnemonic_env: None,
            mnemonic_password_env: None,
        }
    }

    fn assert_prepared_transfer_destination(
        boc: &[u8],
        expected_destination: Address,
        expect_init: bool,
    ) {
        let decoded =
            crate::tlb::Message::from_cell(crate::tvm::deserialize_boc(boc).unwrap()).unwrap();
        match decoded.info {
            crate::tlb::CommonMsgInfo::ExternalIn { dest, .. } => {
                assert_eq!(dest, crate::tlb::MsgAddressInt::std(expected_destination));
            }
            _ => panic!("expected external inbound message"),
        }
        assert_eq!(decoded.init.is_some(), expect_init);
    }

    #[test]
    fn cli_debug_asserts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_liteclient_json_command() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "--rps",
            "5",
            "liteclient",
            "masterchain-info",
            "--ls-index",
            "2",
        ])
        .unwrap();

        assert_eq!(cli.output, OutputFormat::Json);
        assert_eq!(cli.rps.map(NonZeroU32::get), Some(5));
        match cli.command {
            Commands::Liteclient {
                command: LiteClientCommand::MasterchainInfo { ls_index },
            } => assert_eq!(ls_index, 2),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_balancer_global_rps() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--global-rps",
            "10",
            "balancer",
            "masterchain-info",
            "--num-servers",
            "2",
        ])
        .unwrap();

        assert_eq!(cli.global_rps.map(NonZeroU32::get), Some(10));
        match cli.command {
            Commands::Balancer {
                command: BalancerCommand::MasterchainInfo { num_servers },
            } => assert_eq!(num_servers, 2),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_high_level_commands() {
        let block = "0:0x8000000000000000:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let address = "0:3333333333333333333333333333333333333333333333333333333333333333";

        let status = Cli::try_parse_from([
            "tonutils",
            "--num-servers",
            "2",
            "--single",
            "--ls-index",
            "1",
            "status",
        ])
        .unwrap();
        assert!(status.single);
        assert_eq!(status.num_servers, 2);
        assert_eq!(status.ls_index, 1);

        for args in [
            vec!["account", address, "--block", block],
            vec![
                "call", address, "seqno", "--arg", "int:1", "--arg", "null", "--block", block,
            ],
            vec!["call", address, "85143"],
            vec!["transactions", address, "--count", "5"],
            vec!["block", "latest"],
            vec!["block", "get", block],
            vec!["config", "get", "--params", "0,17", "--block", block],
            vec!["config", "get"],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn parses_stack_args() {
        let stack = parse_stack_args(&["int:-5".to_owned(), "null".to_owned()]).unwrap();
        assert_eq!(stack.entries().len(), 2);
        assert!(matches!(stack.entries()[0], TvmStackEntry::Int(_)));
        assert!(matches!(stack.entries()[1], TvmStackEntry::Null));

        let cell = crate::tvm::CellBuilder::new().build().unwrap();
        let boc = hex::encode(crate::tvm::serialize_boc(&cell, false).unwrap());
        assert!(matches!(
            parse_stack_arg(&format!("cell:{boc}")).unwrap(),
            TvmStackEntry::Cell(_)
        ));
        assert!(matches!(
            parse_stack_arg(&format!("slice:{boc}")).unwrap(),
            TvmStackEntry::Slice(_)
        ));

        assert!(parse_stack_arg("bad").is_err());
        assert!(parse_stack_arg("uint:1").is_err());
        assert!(parse_stack_arg("int:not-a-number").is_err());
        assert!(parse_stack_arg("cell:00").is_err());
    }

    #[test]
    fn rejects_zero_rps() {
        assert!(Cli::try_parse_from(["tonutils", "--rps", "0", "liteclient", "time"]).is_err());
    }

    #[test]
    fn rejects_zero_global_rps() {
        assert!(
            Cli::try_parse_from(["tonutils", "--global-rps", "0", "balancer", "status"]).is_err()
        );
    }

    #[test]
    fn raw_input_decodes_hex() {
        let bytes = read_raw_input(&Some("0a0b0c".to_owned()), &None, &None, false).unwrap();
        assert_eq!(bytes, vec![10, 11, 12]);
    }

    #[test]
    fn parses_contract_state_command() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "contract",
            "state",
            "--ls-index",
            "1",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command: ContractCommand::State { ls_index, address },
            } => {
                assert_eq!(ls_index, 1);
                assert!(address.starts_with("0:11"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_contract_run_get_method_by_name() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-get-method",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            "--method",
            "seqno",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command:
                    ContractCommand::RunGetMethod {
                        method, method_id, ..
                    },
            } => {
                assert_eq!(method.as_deref(), Some("seqno"));
                assert_eq!(method_id, None);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_contract_run_get_method_by_id() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-get-method",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            "--method-id",
            "85143",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command:
                    ContractCommand::RunGetMethod {
                        method, method_id, ..
                    },
            } => {
                assert_eq!(method, None);
                assert_eq!(method_id, Some(85143));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_contract_run_abi_get_method() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-abi-get-method",
            "--address",
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            "--abi-file",
            "wallet.abi.json",
            "--contract",
            "Wallet",
            "--method",
            "seqno",
            "--arg",
            "owner=\"0:1111111111111111111111111111111111111111111111111111111111111111\"",
        ])
        .unwrap();

        match cli.command {
            Commands::Contract {
                command: ContractCommand::RunAbiGetMethod(args),
            } => {
                assert_eq!(args.abi_file, "wallet.abi.json");
                assert_eq!(args.contract.as_deref(), Some("Wallet"));
                assert_eq!(args.method.as_deref(), Some("seqno"));
                assert_eq!(args.args.len(), 1);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_wallet_commands() {
        let address = "0:1111111111111111111111111111111111111111111111111111111111111111";
        for args in [
            vec!["wallet", "generate"],
            vec![
                "wallet",
                "address",
                "--version",
                "v4r2",
                "--mnemonic-file",
                "-",
            ],
            vec!["wallet", "seqno", address],
            vec![
                "wallet",
                "prepare-transfer",
                "--mnemonic-env",
                "TON_MNEMONIC",
                "--to",
                address,
                "--amount",
                "1000",
                "--seqno",
                "7",
                "--output",
                "hex",
            ],
            vec![
                "wallet",
                "send",
                "--version",
                "v4r2",
                "--mnemonic-env",
                "TON_MNEMONIC",
                "--to",
                address,
                "--amount",
                "1000",
                "--deploy",
            ],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn parses_tvm_boc_decode_with_tlb_type() {
        let cli = Cli::try_parse_from([
            "tonutils",
            "--output",
            "json",
            "tvm",
            "boc",
            "decode",
            "--hex",
            "b5ee9c72010101010002000000",
            "--tlb",
            "account",
        ])
        .unwrap();

        match cli.command {
            Commands::Tvm {
                command:
                    TvmCommand::Boc {
                        command: BocCommand::Decode { tlb, .. },
                    },
            } => assert_eq!(tlb, Some(KnownTlbType::Account)),
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_tvm_schema_check() {
        let cli = Cli::try_parse_from(["tonutils", "tvm", "schema", "check"]).unwrap();

        match cli.command {
            Commands::Tvm {
                command:
                    TvmCommand::Schema {
                        command: SchemaCommand::Check,
                    },
            } => {}
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parses_new_liteclient_commands() {
        let block = "0:0x8000000000000000:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let account = "0:3333333333333333333333333333333333333333333333333333333333333333";
        let address = account;
        let hash = "4444444444444444444444444444444444444444444444444444444444444444";

        for args in [
            vec!["liteclient", "raw-get-block", "--block", block],
            vec![
                "liteclient",
                "raw-get-block-header",
                "--block",
                block,
                "--with-state-update",
                "--with-value-flow",
                "--with-extra",
                "--with-shard-hashes",
                "--with-prev-blk-signatures",
            ],
            vec![
                "liteclient",
                "get-account-state-typed",
                "--address",
                address,
                "--block",
                block,
            ],
            vec!["liteclient", "raw-get-account-state", "--address", address],
            vec![
                "liteclient",
                "get-account-state-simple",
                "--address",
                address,
            ],
            vec![
                "liteclient",
                "raw-get-shard-info",
                "--block",
                block,
                "--workchain",
                "0",
                "--shard",
                "0x8000000000000000",
                "--exact",
            ],
            vec!["liteclient", "raw-get-all-shards-info", "--block", block],
            vec!["liteclient", "get-all-shards-info-typed", "--block", block],
            vec![
                "liteclient",
                "get-one-transaction-typed",
                "--block",
                block,
                "--account",
                account,
                "--lt",
                "7",
            ],
            vec![
                "liteclient",
                "raw-get-transactions",
                "--account",
                account,
                "--lt",
                "7",
                "--hash",
                hash,
                "--count",
                "3",
            ],
            vec![
                "liteclient",
                "raw-get-block-transactions-ext",
                "--block",
                block,
                "--count",
                "3",
                "--after-account",
                hash,
                "--after-lt",
                "7",
                "--reverse-order",
                "--want-proof",
            ],
            vec![
                "liteclient",
                "run-get-method-typed",
                "--address",
                address,
                "--method",
                "seqno",
            ],
            vec![
                "liteclient",
                "get-config-all-typed",
                "--block",
                block,
                "--with-state-root",
            ],
            vec![
                "liteclient",
                "get-config-params-typed",
                "--block",
                block,
                "--params",
                "0,1,-1",
                "--with-libraries",
            ],
            vec!["liteclient", "get-libraries-typed", "--libraries", hash],
            vec![
                "liteclient",
                "get-libraries-with-proof-typed",
                "--block",
                block,
                "--libraries",
                hash,
                "--mode",
                "1",
            ],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn parses_new_balancer_commands() {
        let block = "0:9223372036854775808:1:1111111111111111111111111111111111111111111111111111111111111111:2222222222222222222222222222222222222222222222222222222222222222";
        let account = "0:3333333333333333333333333333333333333333333333333333333333333333";
        let hash = "4444444444444444444444444444444444444444444444444444444444444444";

        for args in [
            vec!["balancer", "raw-get-block", "--block", block],
            vec!["balancer", "raw-get-block-header", "--block", block],
            vec!["balancer", "get-account-state-typed", "--address", account],
            vec!["balancer", "raw-get-account-state", "--address", account],
            vec!["balancer", "get-account-state-simple", "--address", account],
            vec![
                "balancer",
                "raw-get-shard-info",
                "--block",
                block,
                "--workchain",
                "0",
                "--shard",
                "1",
            ],
            vec!["balancer", "raw-get-all-shards-info", "--block", block],
            vec!["balancer", "get-all-shards-info-typed", "--block", block],
            vec![
                "balancer",
                "get-one-transaction-typed",
                "--block",
                block,
                "--account",
                account,
                "--lt",
                "7",
            ],
            vec![
                "balancer",
                "raw-get-transactions",
                "--account",
                account,
                "--lt",
                "7",
                "--hash",
                hash,
                "--count",
                "3",
            ],
            vec![
                "balancer",
                "raw-get-block-transactions-ext",
                "--block",
                block,
                "--count",
                "3",
            ],
            vec![
                "balancer",
                "run-get-method-typed",
                "--address",
                account,
                "--method-id",
                "85143",
            ],
            vec!["balancer", "get-config-all-typed", "--block", block],
            vec![
                "balancer",
                "get-config-params-typed",
                "--block",
                block,
                "--params",
                "0,1",
            ],
            vec!["balancer", "get-libraries-typed", "--libraries", hash],
            vec![
                "balancer",
                "get-libraries-with-proof-typed",
                "--block",
                block,
                "--libraries",
                hash,
            ],
        ] {
            let mut full = vec!["tonutils"];
            full.extend(args);
            Cli::try_parse_from(full).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_typed_cli_inputs() {
        assert!(parse_block_id_ext("0:1:2:abcd").is_err());
        assert!(parse_block_id_ext("0:1:2:abcd:00").is_err());
        assert!(parse_params("1,,2").is_err());
        assert!(parse_libraries("abcd").is_err());
        assert!(parse_after_transaction(&Some("11".to_owned()), None).is_err());

        assert!(
            Cli::try_parse_from([
                "tonutils",
                "liteclient",
                "run-get-method-typed",
                "--address",
                "0:1111111111111111111111111111111111111111111111111111111111111111",
                "--method",
                "seqno",
                "--method-id",
                "85143",
            ])
            .is_err()
        );
    }

    #[test]
    fn account_state_json_view_contains_lengths_and_raw_state() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let view = account_state_view(crate::tl::response::AccountState {
            id: block.clone(),
            shardblk: block,
            shard_proof: vec![1, 2],
            proof: vec![3],
            state: vec![4, 5, 6],
        });

        assert_eq!(view.shard_proof_len, 2);
        assert_eq!(view.proof_len, 1);
        assert_eq!(view.state.hex, "040506");
    }

    #[test]
    fn best_effort_account_state_accepts_multi_root_proofs() {
        use crate::tlb::TlbSerialize;

        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let account = crate::tlb::Account::None;
        let state = crate::tvm::serialize_boc(&account.to_cell().unwrap(), false).unwrap();
        let proof = hex::decode("b5ee9c72010102020005000100000002aa").unwrap();
        let view = best_effort_account_state_view(
            "0:1111111111111111111111111111111111111111111111111111111111111111",
            crate::tl::response::AccountState {
                id: block.clone(),
                shardblk: block,
                shard_proof: proof.clone(),
                proof,
                state,
            },
        );

        assert_eq!(view.state, "none");
        assert_eq!(view.shard_proof_root_count, Some(2));
        assert_eq!(view.proof_root_count, Some(2));
        assert_eq!(view.shard_proof_root_hashes.len(), 2);
        assert!(view.decode_errors.is_empty());
    }

    #[test]
    fn run_get_method_json_view_decodes_supported_stack() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let result = crate::tl::response::RunMethodResult {
            mode: (),
            id: block.clone(),
            shardblk: block,
            shard_proof: Some(vec![1, 2]),
            proof: None,
            state_proof: Some(vec![3, 4, 5]),
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: Some(TvmStack::new(vec![TvmStackEntry::int(5)]).to_boc().unwrap()),
        };

        let view = run_get_method_view(result, Some("seqno".to_owned()), 85143).unwrap();

        assert_eq!(view.method.as_deref(), Some("seqno"));
        assert_eq!(view.shard_proof_len, 2);
        assert_eq!(view.state_proof_len, 3);
        assert!(view.decoded_stack.is_some());
        assert!(view.result_decode_error.is_none());
    }

    #[test]
    fn cli_abi_value_parser_accepts_supported_json_shapes() {
        use crate::abi::{AbiParameter, AbiType, AbiValue};
        use num_bigint::BigUint;

        let cell = crate::tvm::CellBuilder::new().build().unwrap();
        let boc = hex::encode(crate::tvm::serialize_boc(&cell, false).unwrap());
        let address = "0:1111111111111111111111111111111111111111111111111111111111111111";
        let params = vec![
            AbiParameter {
                name: "amount".to_owned(),
                ty: AbiType::Uint { bits: 64 },
                optional: false,
            },
            AbiParameter {
                name: "meta".to_owned(),
                ty: AbiType::Tuple(vec![
                    AbiParameter {
                        name: "enabled".to_owned(),
                        ty: AbiType::Bool,
                        optional: false,
                    },
                    AbiParameter {
                        name: "payload".to_owned(),
                        ty: AbiType::Bytes,
                        optional: false,
                    },
                ]),
                optional: false,
            },
            AbiParameter {
                name: "owner".to_owned(),
                ty: AbiType::Address,
                optional: false,
            },
            AbiParameter {
                name: "maybe_cell".to_owned(),
                ty: AbiType::Optional(Box::new(AbiType::Cell)),
                optional: false,
            },
            AbiParameter {
                name: "items".to_owned(),
                ty: AbiType::Array(Box::new(AbiType::Uint { bits: 8 })),
                optional: false,
            },
        ];
        let values = parse_abi_named_args(
            &params,
            &[
                "amount=\"0x10\"".to_owned(),
                "meta={\"enabled\":true,\"payload\":\"0x0a0b\"}".to_owned(),
                format!("owner=\"{address}\""),
                format!("maybe_cell=\"{boc}\""),
                "items=[1,\"0x02\"]".to_owned(),
            ],
        )
        .unwrap();

        assert_eq!(values[0], AbiValue::Uint(BigUint::from(16u8)));
        assert!(matches!(values[1], AbiValue::Tuple(_)));
        assert!(matches!(values[2], AbiValue::Address(_)));
        assert!(matches!(values[3], AbiValue::Optional(Some(_))));
        assert!(matches!(values[4], AbiValue::Array(_)));
        assert!(
            parse_abi_value(
                &AbiType::Map {
                    key: Box::new(AbiType::Uint { bits: 8 }),
                    value: Box::new(AbiType::Uint { bits: 8 }),
                },
                &json!({})
            )
            .is_err()
        );
    }

    #[test]
    fn cli_abi_get_method_view_decodes_structured_outputs() {
        use crate::abi::{
            AbiContract, AbiFunction, AbiFunctionKind, AbiParameter, AbiSelector, AbiType,
        };
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let result = crate::tl::response::RunMethodResult {
            mode: (),
            id: block.clone(),
            shardblk: block,
            shard_proof: None,
            proof: None,
            state_proof: None,
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: Some(TvmStack::new(vec![TvmStackEntry::int(5)]).to_boc().unwrap()),
        };
        let function = AbiFunction {
            name: "seqno".to_owned(),
            kind: AbiFunctionKind::GetMethod,
            selector: AbiSelector::MethodId(85143),
            inputs: Vec::new(),
            outputs: vec![AbiParameter {
                name: "value".to_owned(),
                ty: AbiType::Uint { bits: 32 },
                optional: false,
            }],
        };
        let contract = AbiContract {
            name: "Wallet".to_owned(),
            methods: vec![function.clone()],
            events: Vec::new(),
        };

        let view = abi_get_method_view(result, &contract, &function, 85143).unwrap();

        assert_eq!(view.contract, "Wallet");
        assert_eq!(view.method, "seqno");
        assert_eq!(view.outputs[0].name, "value");
        assert_eq!(view.outputs[0].abi_type, "uint32");
        assert_eq!(view.outputs[0].value["decimal"], "5");
    }

    #[test]
    fn wallet_send_deploy_treats_missing_seqno_stack_as_zero() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let result = crate::tl::response::RunMethodResult {
            mode: (),
            id: block.clone(),
            shardblk: block,
            shard_proof: None,
            proof: None,
            state_proof: None,
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: None,
        };

        assert_eq!(
            seqno_from_stack_or_deploy_zero(result.clone(), true).unwrap(),
            0
        );
        assert!(seqno_from_stack_or_deploy_zero(result, false).is_err());
    }

    #[test]
    fn wallet_send_deploy_seqno_fallback_is_missing_stack_only() {
        let block = BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno: 1,
            root_hash: crate::tl::Int256([1; 32]),
            file_hash: crate::tl::Int256([2; 32]),
        };
        let result = crate::tl::response::RunMethodResult {
            mode: (),
            id: block.clone(),
            shardblk: block,
            shard_proof: None,
            proof: None,
            state_proof: None,
            init_c7: None,
            lib_extras: None,
            exit_code: 0,
            result: Some(TvmStack::new(vec![TvmStackEntry::Null]).to_boc().unwrap()),
        };

        assert!(seqno_from_stack_or_deploy_zero(result, true).is_err());
    }

    #[test]
    fn wallet_cli_preserves_default_wallet_ids_for_networks() {
        assert_eq!(
            wallet_id_for_cli(WalletVersionArg::V4R2, Network::Mainnet, 0, None).unwrap(),
            WALLET_V4R2_DEFAULT_ID
        );
        assert_eq!(
            wallet_id_for_cli(WalletVersionArg::V4R2, Network::Testnet, 0, None).unwrap(),
            WALLET_V4R2_DEFAULT_ID
        );
        assert_eq!(
            wallet_id_for_cli(WalletVersionArg::V5R1, Network::Mainnet, 0, None).unwrap(),
            WalletV5R1WalletId::client(MAINNET_GLOBAL_ID, 0, 0, 0)
                .pack()
                .unwrap()
        );
        assert_eq!(
            wallet_id_for_cli(WalletVersionArg::V5R1, Network::Testnet, 0, None).unwrap(),
            WalletV5R1WalletId::client(TESTNET_GLOBAL_ID, 0, 0, 0)
                .pack()
                .unwrap()
        );
    }

    #[test]
    fn wallet_cli_prepared_transfers_decode_to_derived_wallet_address() {
        let mnemonic = fixture_mnemonic();

        for (network, version) in [
            (Network::Mainnet, WalletVersionArg::V5R1),
            (Network::Testnet, WalletVersionArg::V5R1),
            (Network::Mainnet, WalletVersionArg::V4R2),
            (Network::Testnet, WalletVersionArg::V4R2),
        ] {
            for deploy in [false, true] {
                let args = transfer_args(version, deploy);
                let wallet_id =
                    wallet_id_for_cli(version, network, args.workchain, args.wallet_id).unwrap();
                let expected =
                    wallet_address_view(version, args.workchain, wallet_id, mnemonic.public_key())
                        .unwrap();

                let (boc, view) = build_wallet_transfer(network, &args, &mnemonic, 7).unwrap();

                assert_eq!(view.address.wallet_id, wallet_id);
                assert_eq!(view.address.address, expected.address);
                assert_eq!(view.deploy, deploy);
                assert_prepared_transfer_destination(
                    &boc,
                    Address::from_str(&expected.address).unwrap(),
                    deploy,
                );
            }
        }
    }
}
