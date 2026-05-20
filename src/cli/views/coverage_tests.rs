use super::*;
use crate::wallet::WALLET_V5R1_MAINNET_DEFAULT_ID;
use num_bigint::BigUint;

fn block_view(seqno: i32) -> BlockIdExtView {
    BlockIdExtView {
        workchain: -1,
        shard: i64::MIN,
        seqno,
        root_hash: "11".repeat(32),
        file_hash: "22".repeat(32),
    }
}

fn block_id(seqno: i32) -> BlockIdExt {
    BlockIdExt {
        workchain: -1,
        shard: i64::MIN,
        seqno,
        root_hash: Int256([0x11; 32]),
        file_hash: Int256([0x22; 32]),
    }
}

fn raw(bytes: &[u8]) -> RawBytesView {
    raw_bytes_view(bytes)
}

fn empty_cell() -> Arc<Cell> {
    Builder::new().build().unwrap()
}

#[test]
fn serializes_cli_view_shapes() {
    let block = block_view(10);
    let raw = raw(&[1, 2, 3]);
    let stack = TvmStackView {
        entries: vec![
            TvmStackEntryView::Null,
            TvmStackEntryView::Int {
                decimal: "-1".to_owned(),
            },
            TvmStackEntryView::Cell { boc: raw.clone() },
            TvmStackEntryView::Slice { boc: raw.clone() },
            TvmStackEntryView::Tuple { entries: vec![] },
            TvmStackEntryView::List { entries: vec![] },
            TvmStackEntryView::Unsupported { raw: raw.clone() },
        ],
    };

    let views = vec![
        json!(CellView {
            bits: 0,
            refs: 0,
            exotic: false,
            level: 0,
            depth: 0,
            hash: "00".repeat(32),
        }),
        json!(BocDecodeView {
            raw: raw.clone(),
            root: cell_view(&empty_cell()),
            tlb_type: Some("account".to_owned()),
            tlb: Some(json!({"type": "none"})),
            proof_verified: Some(false),
        }),
        json!(SchemaCheckView {
            schema: "block.tlb",
            constructors: 7,
            generated_matches: true,
        }),
        json!(RunGetMethodView {
            block: block_view(1),
            shard_block: block_view(2),
            method: Some("seqno".to_owned()),
            method_id: 85143,
            exit_code: 0,
            shard_proof_len: 1,
            proof_len: 2,
            state_proof_len: 3,
            result: Some(raw.clone()),
            decoded_stack: Some(stack),
            result_decode_error: None,
        }),
        json!(AccountStateView {
            block: block_view(3),
            shard_block: block_view(4),
            shard_proof_len: 5,
            proof_len: 6,
            state: raw.clone(),
        }),
        json!(BalancerStatusView {
            total_peers: 3,
            alive_peers: 2,
            archival_peers: 1,
        }),
        json!(StatusView {
            network: NetworkView { name: "testnet" },
            backend: BackendView {
                mode: "single",
                ls_index: Some(0),
                num_servers: None,
            },
            latest: block_view(5),
            peers: Some(BalancerStatusView {
                total_peers: 1,
                alive_peers: 1,
                archival_peers: 0,
            }),
        }),
        json!(BestEffortAccountStateView {
            address: "0:".to_owned() + &"11".repeat(32),
            block: block_view(6),
            shard_block: block_view(7),
            state: "active".to_owned(),
            balance: Some("123".to_owned()),
            last_transaction_lt: Some(42),
            last_transaction_hash: Some("33".repeat(32)),
            shard_proof_len: 1,
            proof_len: 2,
            state_len: 3,
            shard_proof_root_count: Some(1),
            proof_root_count: Some(1),
            shard_proof_root_hash: Some("44".repeat(32)),
            proof_root_hash: Some("55".repeat(32)),
            shard_proof_root_hashes: vec!["44".repeat(32)],
            proof_root_hashes: vec!["55".repeat(32)],
            state_root_hash: Some("66".repeat(32)),
            account: Some(json!({"type": "none"})),
            shard_account: Some(json!({"last_trans_lt": 42})),
            decode_errors: vec!["decode failed".to_owned()],
        }),
        json!(HighLevelCallView {
            address: "0:".to_owned() + &"11".repeat(32),
            block: block_view(8),
            shard_block: block_view(9),
            method: None,
            method_id: 1,
            exit_code: 0,
            stack: None,
            decode_errors: vec![],
        }),
        json!(HighLevelTransactionsView {
            address: "0:".to_owned() + &"11".repeat(32),
            count: 1,
            start_lt: Some(10),
            start_hash: Some("77".repeat(32)),
            ids: vec![block_view(10)],
            transactions: vec![json!({"lt": 10})],
            decode_errors: vec![],
        }),
        json!(WalletAddressView {
            version: WalletVersionArg::V5R1,
            workchain: 0,
            wallet_id: WALLET_V5R1_MAINNET_DEFAULT_ID,
            address: "0:".to_owned() + &"88".repeat(32),
            bounceable: "EQ...".to_owned(),
            non_bounceable: "UQ...".to_owned(),
        }),
        json!(WalletGenerateView {
            mnemonic: "abandon ".repeat(23) + "about",
            public_key: "99".repeat(32),
            v5r1: WalletAddressView {
                version: WalletVersionArg::V5R1,
                workchain: 0,
                wallet_id: WALLET_V5R1_MAINNET_DEFAULT_ID,
                address: "0:".to_owned() + &"88".repeat(32),
                bounceable: "EQ...".to_owned(),
                non_bounceable: "UQ...".to_owned(),
            },
            v4r2: WalletAddressView {
                version: WalletVersionArg::V4R2,
                workchain: 0,
                wallet_id: WALLET_V4R2_DEFAULT_ID,
                address: "0:".to_owned() + &"99".repeat(32),
                bounceable: "EQ...".to_owned(),
                non_bounceable: "UQ...".to_owned(),
            },
        }),
        json!(WalletSeqnoView {
            address: "0:".to_owned() + &"aa".repeat(32),
            seqno: 11,
            block: block_view(11),
        }),
        json!(WalletSendView {
            prepared: WalletPreparedTransferView {
                version: WalletVersionArg::V5R1,
                address: WalletAddressView {
                    version: WalletVersionArg::V5R1,
                    workchain: 0,
                    wallet_id: WALLET_V5R1_MAINNET_DEFAULT_ID,
                    address: "0:".to_owned() + &"88".repeat(32),
                    bounceable: "EQ...".to_owned(),
                    non_bounceable: "UQ...".to_owned(),
                },
                to: "0:".to_owned() + &"bb".repeat(32),
                amount: 1_000,
                seqno: 12,
                valid_until: 13,
                deploy: true,
                boc: raw,
            },
            status: 1,
        }),
        json!(MasterchainInfoView {
            last: block,
            state_root_hash: "cc".repeat(32),
            init_workchain: -1,
            init_root_hash: "dd".repeat(32),
            init_file_hash: "ee".repeat(32),
        }),
        json!(VersionView {
            mode: 1,
            version: 2,
            capabilities: 3,
            now: 4,
        }),
        json!(TimeView { now: 5 }),
    ];

    assert!(views.iter().all(Value::is_object));
}

#[test]
fn parses_cli_value_helpers_and_errors() {
    let block = "-1:0x8000000000000000:7:".to_owned() + &"11".repeat(32) + ":" + &"22".repeat(32);
    assert_eq!(parse_block_id_ext(&block).unwrap().seqno, 7);
    assert_eq!(parse_i64_decimal_or_hex("0x7f").unwrap(), 127);
    assert_eq!(parse_i64_decimal_or_hex("-2").unwrap(), -2);
    assert_eq!(parse_u64_decimal_or_hex("0X10").unwrap(), 16);
    assert_eq!(parse_params("0, 17").unwrap(), vec![0, 17]);
    assert_eq!(
        parse_libraries(&("11".repeat(32) + "," + &"22".repeat(32)))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        parse_account_id(&("0:".to_owned() + &"33".repeat(32)))
            .unwrap()
            .workchain,
        0
    );
    assert_eq!(parse_method_ref("0x14").unwrap(), (None, 20));
    assert_eq!(
        parse_method_ref("seqno").unwrap().0.as_deref(),
        Some("seqno")
    );
    assert_eq!(
        parse_after_transaction(&Some("44".repeat(32)), Some(5))
            .unwrap()
            .unwrap()
            .lt,
        5
    );
    assert_eq!(
        latest_or_explicit_block(None, block_id(9)).unwrap().seqno,
        9
    );

    assert!(parse_block_id_ext("bad").is_err());
    assert!(parse_int256("00").is_err());
    assert!(parse_account_id("0:1:2").is_err());
    assert!(parse_params("1,,2").is_err());
    assert!(parse_after_transaction(&Some("44".repeat(32)), None).is_err());
}

#[test]
fn converts_tlb_helpers_to_json_values() {
    let cell = empty_cell();
    assert_eq!(cell_value(&cell)["refs"], 0);
    assert_eq!(anycast_value(None), Value::Null);
    assert_eq!(
        anycast_value(Some(&crate::tlb::Anycast {
            depth: 8,
            rewrite_pfx: vec![0xaa],
        }))["depth"],
        8
    );

    let std_addr = crate::tlb::MsgAddressInt::std(Address::new(0, [0x11; 32]));
    assert_eq!(msg_address_int_value(&std_addr)["type"], "std");
    let var_addr = crate::tlb::MsgAddressInt::Var {
        anycast: None,
        workchain_id: -1,
        address: vec![0xf0],
        bit_len: 4,
    };
    assert_eq!(msg_address_int_value(&var_addr)["type"], "var");

    assert_eq!(
        account_state_value(&crate::tlb::AccountState::Uninit)["type"],
        "uninit"
    );
    assert_eq!(
        account_state_value(&crate::tlb::AccountState::Frozen {
            state_hash: [0x22; 32],
        })["type"],
        "frozen"
    );
    assert_eq!(
        account_state_value(&crate::tlb::AccountState::Active {
            state_init: crate::tlb::StateInit::empty(),
        })["type"],
        "active"
    );
    assert_eq!(
        account_value(&crate::tlb::Account::None),
        json!({ "type": "none" })
    );
    assert_eq!(
        currency_collection_value(&crate::tlb::CurrencyCollection::grams(crate::tlb::Grams(
            BigUint::from(7u32),
        )))["grams"],
        "7"
    );

    for (status, name) in [
        (crate::tlb::AccountStatus::Uninit, "uninit"),
        (crate::tlb::AccountStatus::Frozen, "frozen"),
        (crate::tlb::AccountStatus::Active, "active"),
        (crate::tlb::AccountStatus::Nonexist, "nonexist"),
    ] {
        assert_eq!(account_status_name(status), name);
    }

    for (state, name) in [
        (crate::liteclient::boc::SimpleAccountState::None, "none"),
        (crate::liteclient::boc::SimpleAccountState::Uninit, "uninit"),
        (crate::liteclient::boc::SimpleAccountState::Frozen, "frozen"),
        (crate::liteclient::boc::SimpleAccountState::Active, "active"),
    ] {
        assert_eq!(simple_account_state_name(&state), name);
    }

    assert_eq!(
        shard_state_value(&crate::tlb::ShardState::Unsplit {
            payload: cell.clone(),
        })["type"],
        "unsplit"
    );
    assert_eq!(
        shard_state_value(&crate::tlb::ShardState::Split {
            left: cell.clone(),
            right: cell.clone(),
        })["type"],
        "split"
    );
    assert_eq!(
        config_params_value(&crate::tlb::ConfigParams {
            config_addr: [0x33; 32],
            config: cell.clone(),
        })["config_addr"],
        "33".repeat(32)
    );

    let mut libraries = std::collections::HashMap::new();
    libraries.insert(Int256([0x44; 32]), Some(cell.clone()));
    libraries.insert(Int256([0x55; 32]), None);
    assert_eq!(libraries_value(&libraries).as_object().unwrap().len(), 2);

    let simple = crate::liteclient::boc::SimpleAccount {
        block_id: block_id(1),
        shard_block_id: block_id(2),
        last_transaction_lt: Some(3),
        last_transaction_hash: Some([0x66; 32]),
        state: crate::liteclient::boc::SimpleAccountState::None,
        account: Some(crate::tlb::Account::None),
    };
    assert_eq!(simple_account_value(&simple)["state"], "none");
}
