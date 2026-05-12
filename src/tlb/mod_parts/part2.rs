
#[cfg(test)]
mod offline_fixture_tests {
    use super::*;
    use crate::tvm::{
        Address, BitKey, Builder, Cell, HashmapAug, HashmapAugE, HashmapAugLeaf, HashmapE, Slice,
        base64_to_boc, boc_to_hex, hex_to_boc,
    };
    use num_bigint::BigUint;
    use std::fmt::Debug;
    use std::sync::Arc;

    struct TlbFixture {
        name: &'static str,
        source: &'static str,
        encoded: FixtureEncoding,
        expected_root_hash: &'static str,
        decoded_type: &'static str,
    }

    enum FixtureEncoding {
        Hex(&'static str),
        Base64(&'static str),
    }

    fn assert_fixture<T>(fixture: &TlbFixture, expected: &T)
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + Debug,
    {
        assert!(!fixture.name.is_empty());
        assert!(!fixture.source.is_empty());
        assert!(!fixture.decoded_type.is_empty());

        let cell = fixture_cell(fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);

        let decoded = T::from_cell(cell.clone()).unwrap();
        assert_eq!(&decoded, expected, "{}", fixture.name);

        let canonical_cell = decoded.to_cell().unwrap();
        assert_eq!(canonical_cell.hash(), cell.hash(), "{}", fixture.name);
        assert_eq!(
            boc_to_hex(&canonical_cell, false).unwrap(),
            boc_to_hex(&cell, false).unwrap(),
            "{}",
            fixture.name
        );
    }

    fn assert_trailing_data_is_rejected<T>(fixture: &TlbFixture)
    where
        T: TlbDeserialize + Debug,
    {
        let cell = fixture_cell(fixture);
        let mut builder = Builder::new();
        builder.store_bits(cell.data(), cell.bit_len()).unwrap();
        for reference in cell.references() {
            builder.store_ref(reference.clone()).unwrap();
        }
        builder.store_bit(true).unwrap();

        let err = T::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(
            matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }),
            "{}",
            fixture.name
        );
    }

    fn fixture_cell(fixture: &TlbFixture) -> Arc<Cell> {
        match fixture.encoded {
            FixtureEncoding::Hex(hex) => hex_to_boc(hex).unwrap(),
            FixtureEncoding::Base64(base64) => base64_to_boc(base64).unwrap(),
        }
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn account_address() -> MsgAddressInt {
        MsgAddressInt::std(std_address(0x11))
    }

    fn message_fixture_value() -> Message {
        Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: account_address(),
                import_fee: Grams::from(1),
            },
            init: Some(Either::Right(StateInit {
                code: Some(cell_with_bits(&[0xA5], 8)),
                ..StateInit::empty()
            })),
            body: Either::Right(cell_with_bits(&[0x80], 1)),
        }
    }

    fn relaxed_message_fixture_value() -> MessageRelaxed {
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: false,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(std_address(0x22)),
                value: CurrencyCollection::grams(Grams::from(7)),
                extra_flags: BigUint::from(2u8),
                fwd_fee: Grams::from(3),
                created_lt: 4,
                created_at: 5,
            },
            init: None,
            body: Either::Right(cell_with_bits(&[0xAD, 0x80], 9)),
        }
    }

    fn currency_collection_fixture_value() -> CurrencyCollection {
        let mut other = HashmapE::new(32);
        other
            .insert_bit_key(BitKey::from_u64(7, 32).unwrap(), BigUint::from(42u8))
            .unwrap();
        CurrencyCollection {
            grams: Grams::from(123),
            other,
        }
    }

    fn state_init_fixture_value() -> StateInit {
        StateInit {
            fixed_prefix_length: Some(5),
            special: Some(TickTock {
                tick: true,
                tock: false,
            }),
            code: Some(cell_with_bits(&[0xAA], 8)),
            data: Some(cell_with_bits(&[0xBC], 6)),
            library: None,
        }
    }

    fn storage_phase() -> TrStoragePhase {
        TrStoragePhase {
            storage_fees_collected: Grams::from(7),
            storage_fees_due: Some(Grams::from(8)),
            status_change: AccStatusChange::Frozen,
        }
    }

    fn hash_update() -> HashUpdateAccount {
        HashUpdateAccount {
            old_hash: [0xAA; 32],
            new_hash: [0xBB; 32],
        }
    }

    fn storage_info() -> StorageInfo {
        StorageInfo {
            used: StorageUsed::new(BigUint::from(2u8), BigUint::from(128u16)),
            last_paid: 1_700_000_001,
            due_payment: Some(Grams::from(4)),
            extra: StorageExtraInfo::Info {
                dict_hash: [0xCC; 32],
            },
        }
    }

    fn account_storage() -> AccountStorage {
        AccountStorage {
            last_trans_lt: 11,
            balance: CurrencyCollection::grams(Grams::from(100)),
            state: AccountState::Active {
                state_init: StateInit::empty(),
            },
        }
    }

    fn account_fixture_value() -> Account {
        Account::Full {
            addr: account_address(),
            storage_stat: storage_info(),
            storage: account_storage(),
        }
    }

    fn transaction_descr_fixture_value() -> TransactionDescr {
        TransactionDescr::Storage {
            storage_ph: storage_phase(),
        }
    }

    fn transaction_fixture_value() -> Transaction {
        Transaction {
            account_addr: [0x10; 32],
            lt: 7,
            prev_trans_hash: [0x20; 32],
            prev_trans_lt: 6,
            now: 1_700_000_000,
            outmsg_cnt: 0,
            orig_status: AccountStatus::Active,
            end_status: AccountStatus::Active,
            in_msg: None,
            out_msgs: HashmapE::new(15),
            total_fees: CurrencyCollection::grams(Grams::from(3)),
            state_update: hash_update(),
            description: transaction_descr_fixture_value(),
        }
    }

    fn depth_balance(split_depth: u8, grams: u64) -> DepthBalanceInfo {
        DepthBalanceInfo {
            split_depth,
            balance: CurrencyCollection::grams(Grams::from(grams)),
        }
    }

    fn shard_accounts_fixture_value() -> ShardAccounts {
        let shard_account = ShardAccount {
            account: account_fixture_value(),
            last_trans_hash: [0x44; 32],
            last_trans_lt: 12,
        };
        let root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x11; 32], 256).unwrap(),
                value: shard_account,
                extra: depth_balance(7, 100),
            }],
            depth_balance(7, 100),
        )
        .unwrap();
        ShardAccounts {
            accounts: HashmapAugE::with_root(256, root, depth_balance(7, 100)).unwrap(),
        }
    }

    fn account_block_fixture_value() -> AccountBlock {
        let root = HashmapAug::from_entries(
            64,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(7, 64).unwrap(),
                value: transaction_fixture_value(),
                extra: CurrencyCollection::grams(Grams::from(8)),
            }],
            CurrencyCollection::grams(Grams::from(8)),
        )
        .unwrap();
        AccountBlock {
            account_addr: [0x55; 32],
            transactions: root,
            state_update: hash_update(),
        }
    }

    fn shard_account_blocks_fixture_value() -> ShardAccountBlocks {
        let root = HashmapAug::from_entries(
            256,
            vec![HashmapAugLeaf {
                key: BitKey::from_bits(vec![0x22; 32], 256).unwrap(),
                value: account_block_fixture_value(),
                extra: CurrencyCollection::grams(Grams::from(8)),
            }],
            CurrencyCollection::grams(Grams::from(8)),
        )
        .unwrap();
        ShardAccountBlocks {
            blocks: HashmapAugE::with_root(256, root, CurrencyCollection::grams(Grams::from(8)))
                .unwrap(),
        }
    }

    #[test]
    fn message_account_and_transaction_offline_fixtures_roundtrip() {
        const SOURCE: &str = "synthetic schema-derived offline fixture from implemented TL-B model";

        let message = TlbFixture {
            name: "message-with-referenced-state-init-and-body",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010104010032030002a5010124000001c0024788002222222222222222222222222222222222222222222222222222222222222222203e0102",
            ),
            expected_root_hash: "ae43183ebb6674776cf91aa612ed19a6a6f5ab5199ee87eaf36ec62e4ff323e3",
            decoded_type: "Message Any",
        };
        assert_fixture(&message, &message_fixture_value());
        assert_trailing_data_is_rejected::<Message>(&message);

        let relaxed_message = TlbFixture {
            name: "relaxed-message-with-referenced-body",
            source: SOURCE,
            encoded: FixtureEncoding::Base64(
                "te6ccgEBAgEAOgEAA63AAWZCABERERERERERERERERERERERERERERERERERERERERERCDhAhAwAAAAAAAAAEAAAABUA",
            ),
            expected_root_hash: "fdd15c56139da31cacbc9ab2e7435726d89303e6be82cc259e515e3dff8f548c",
            decoded_type: "MessageRelaxed Any",
        };
        assert_fixture(&relaxed_message, &relaxed_message_fixture_value());
        assert_trailing_data_is_rejected::<MessageRelaxed>(&relaxed_message);

        let state_init = TlbFixture {
            name: "state-init-with-prefix-special-code-data",
            source: SOURCE,
            encoded: FixtureEncoding::Hex("b5ee9c7201010301000c020002aa0001be020397680001"),
            expected_root_hash: "33ef718f8d73687800d7c90a5202b4f12703fdd38cdfbf0486a8be78828fe51b",
            decoded_type: "StateInit",
        };
        assert_fixture(&state_init, &state_init_fixture_value());

        let currency = TlbFixture {
            name: "currency-collection-with-extra-currency",
            source: SOURCE,
            encoded: FixtureEncoding::Hex("b5ee9c7201010201000e01000da0000000070954010317bc00"),
            expected_root_hash: "467a118b2a65ebfd9e10fbfd1aa44af31e356145073abb735f12d5b1b07df88c",
            decoded_type: "CurrencyCollection",
        };
        assert_fixture(&currency, &currency_collection_fixture_value());

        let transaction = TlbFixture {
            name: "storage-only-transaction",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101040100aa03000120008272aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb00071107884503b1710101010101010101010101010101010101010101010101010101010101010100000000000000007202020202020202020202020202020202020202020202020202020202020202000000000000000066553f100000142068000102",
            ),
            expected_root_hash: "cdc9b5675d0c34623ccab0b8c28d58aadcfc137634833cee1ea35ba127ecd916",
            decoded_type: "Transaction",
        };
        assert_fixture(&transaction, &transaction_fixture_value());
        assert_trailing_data_is_rejected::<Transaction>(&transaction);

        let transaction_descr = TlbFixture {
            name: "storage-transaction-description",
            source: SOURCE,
            encoded: FixtureEncoding::Base64("te6ccgEBAQEABgAABxEHiEU="),
            expected_root_hash: "c69351ef516847eb1e5bd23f96058ea43510a14f626b189ffdd910ba1e99fbdd",
            decoded_type: "TransactionDescr",
        };
        assert_fixture(&transaction_descr, &transaction_descr_fixture_value());

        let account = TlbFixture {
            name: "full-active-account",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101010100570000a9c001111111111111111111111111111111111111111111111111111111111111111204600e66666666666666666666666666666666666666666666666666666666666666632a9f880c410000000000000002c59104",
            ),
            expected_root_hash: "c11a6be3cd1afdb472d4cc26c62dce6fa6bb9ebd2c0660f12025c03e00dc2595",
            decoded_type: "Account",
        };
        assert_fixture(&account, &account_fixture_value());
    }

    #[test]
    fn augmented_account_collection_offline_fixtures_roundtrip() {
        const SOURCE: &str = "synthetic schema-derived offline fixture from implemented TL-B model";

        let shard_accounts = TlbFixture {
            name: "shard-accounts-single-entry",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101030100ac0200a9c001111111111111111111111111111111111111111111111111111111111111111204600e66666666666666666666666666666666666666666666666666666666666666632a9f880c410000000000000002c591040197a00222222222222222222222222222222222222222222222222222222222222222271642222222222222222222222222222222222222222222222222222222222222222000000000000000640001059c591001",
            ),
            expected_root_hash: "21f2fe5665f466ab2e9d52c223da658af42d45beba9e0664df21b9b873361fa3",
            decoded_type: "ShardAccounts",
        };
        assert_fixture(&shard_accounts, &shard_accounts_fixture_value());

        let account_block = TlbFixture {
            name: "account-block-single-transaction",
            source: SOURCE,
            encoded: FixtureEncoding::Hex(
                "b5ee9c720101050100da04000120008272aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb00071107884503b1710101010101010101010101010101010101010101010101010101010101010100000000000000007202020202020202020202020202020202020202020202020202020202020202000000000000000066553f100000142068000102025755555555555555555555555555555555555555555555555555555555555555555a00000000000000003884200301",
            ),
            expected_root_hash: "397ab2e4d9d18889064c7dc44db29470a30588595e1dc6b93ddfa5822821442b",
            decoded_type: "AccountBlock",
        };
        assert_fixture(&account_block, &account_block_fixture_value());

        let shard_account_blocks = TlbFixture {
            name: "shard-account-blocks-single-entry",
            source: SOURCE,
            encoded: FixtureEncoding::Base64(
                "te6ccgECBgEAAQIFAAEgAIJyqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7u7uwAHEQeIRQOxcQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEAAAAAAAAAAHICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAAAAAAAAAABmVT8QAAAUIGgAAQICnaAEREREREREREREREREREREREREREREREREREREREREREIQVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVaAAAAAAAAAAA4hCADAQEDiEIE",
            ),
            expected_root_hash: "b1ebb413e70b1bde953a55e8e8ac51899ac25adb28f3e02aac97ad5b2fa9ace7",
            decoded_type: "ShardAccountBlocks",
        };
        assert_fixture(&shard_account_blocks, &shard_account_blocks_fixture_value());
    }

    #[test]
    fn hashmap_e_fixture_preserves_canonical_root_reference_and_labels() {
        let fixture = TlbFixture {
            name: "hashmap-e-two-entry-labels",
            source: "synthetic schema-derived offline fixture for HashmapE 4 uint8",
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010104010011030003d00c0003d01402014800010101c002",
            ),
            expected_root_hash: "9c02490e70a529c7242d63c3e85f273d8080b37c64abf8ebc2bcbf8713dc6db9",
            decoded_type: "HashmapE 4 uint8",
        };
        let cell = fixture_cell(&fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);
        assert_eq!(cell.reference_count(), 1);

        let mut slice = Slice::new(cell.clone());
        assert!(slice.load_bit().unwrap());
        let decoded = Slice::new(cell)
            .load_hashmap_e_with(4, |slice| slice.load_uint::<u8>())
            .unwrap();
        let entries: Vec<_> = decoded
            .iter()
            .map(|(key, value)| (key.to_u64().unwrap(), *value))
            .collect();
        assert_eq!(entries, vec![(0, 1), (4, 2)]);
    }

    #[test]
    fn hashmap_aug_e_fixtures_preserve_top_and_fork_extras() {
        let empty: HashmapAugE<u64, u64> = HashmapAugE::empty(4, 88);
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &empty,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();
        let cell = builder.build().unwrap();
        assert_eq!(cell.reference_count(), 0);
        let mut slice = Slice::new(cell);
        let decoded_empty = slice
            .load_hashmap_aug_e_with(
                4,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        assert!(decoded_empty.is_empty());
        assert_eq!(*decoded_empty.extra(), 88);
        ensure_empty(&slice).unwrap();

        let fixture = TlbFixture {
            name: "hashmap-aug-e-three-entry-extras",
            source: "synthetic schema-derived offline fixture for HashmapAugE 4 uint8 uint8",
            encoded: FixtureEncoding::Hex(
                "b5ee9c72010106010020050005d0500c0005d0a0140203136000010005b83c070203136002030103ac4004",
            ),
            expected_root_hash: "6ad8187666c7eef33e1fa3281cc4e18fb0bb9793c11388864bd319c84a1d0612",
            decoded_type: "HashmapAugE 4 uint8 uint8",
        };
        let cell = fixture_cell(&fixture);
        assert_eq!(hex::encode(cell.hash()), fixture.expected_root_hash);
        assert_eq!(cell.reference_count(), 1);

        let mut slice = Slice::new(cell);
        let decoded = slice
            .load_hashmap_aug_e_with(
                4,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        let root = decoded.root().unwrap();
        let leaves: Vec<_> = root
            .iter()
            .map(|(key, value, extra)| (key.to_u64().unwrap(), *value, *extra))
            .collect();
        assert_eq!(leaves, vec![(0, 1, 10), (4, 2, 20), (12, 3, 30)]);
        assert_eq!(*decoded.extra(), 88);
        assert!(root.fork_extras().iter().all(|fork| fork.extra == 77));
        ensure_empty(&slice).unwrap();
    }
}

#[cfg(test)]
mod phase1_checked_fixture_tests {
    use super::*;
    use crate::tvm::{Address, Builder, Cell, HashmapE, boc_to_hex, hex_to_boc};
    use num_bigint::BigUint;
    use serde::Deserialize;
    use std::fmt::Debug;
    use std::sync::Arc;

    #[derive(Debug, Deserialize)]
    struct FixtureSet {
        schema_revision: String,
        fixtures: Vec<Fixture>,
    }

    #[derive(Debug, Deserialize)]
    struct Fixture {
        name: String,
        source: String,
        capture_date: String,
        upstream_commit_or_endpoint: String,
        decoded_type: String,
        root_hash: String,
        canonical_reserialization: String,
        boc_hex: String,
    }

    fn fixture_set(json: &str) -> FixtureSet {
        let set: FixtureSet = serde_json::from_str(json).unwrap();
        assert!(!set.schema_revision.is_empty());
        assert!(!set.fixtures.is_empty());
        set
    }

    fn assert_fixture<T>(fixture: &Fixture, expected_type: &str, expected_value: T)
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + Debug,
    {
        assert!(!fixture.name.is_empty());
        assert!(!fixture.source.is_empty());
        assert!(!fixture.capture_date.is_empty());
        assert!(!fixture.upstream_commit_or_endpoint.is_empty());
        assert_eq!(fixture.decoded_type, expected_type);
        assert!(
            fixture
                .canonical_reserialization
                .contains("canonical BoC without index table or CRC32")
        );

        let cell = hex_to_boc(&fixture.boc_hex).unwrap();
        assert_eq!(
            hex::encode(cell.hash()),
            fixture.root_hash,
            "{}",
            fixture.name
        );

        let decoded = T::from_cell(cell.clone()).unwrap();
        assert_eq!(decoded, expected_value, "{}", fixture.name);
        assert_eq!(
            boc_to_hex(&decoded.to_cell().unwrap(), false).unwrap(),
            fixture.boc_hex,
            "{}",
            fixture.name
        );
    }

    fn find<'a>(set: &'a FixtureSet, name: &str) -> &'a Fixture {
        set.fixtures
            .iter()
            .find(|fixture| fixture.name == name)
            .unwrap_or_else(|| panic!("missing fixture {name}"))
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn account_address() -> MsgAddressInt {
        MsgAddressInt::std(std_address(0x11))
    }

    fn message_fixture_value() -> Message {
        Message {
            info: CommonMsgInfo::ExternalIn {
                src: MsgAddressExt::None,
                dest: account_address(),
                import_fee: Grams::from(1),
            },
            init: Some(Either::Right(StateInit {
                code: Some(cell_with_bits(&[0xA5], 8)),
                ..StateInit::empty()
            })),
            body: Either::Right(cell_with_bits(&[0x80], 1)),
        }
    }

    fn relaxed_message_fixture_value() -> MessageRelaxed {
        MessageRelaxed {
            info: CommonMsgInfoRelaxed::Internal {
                ihr_disabled: true,
                bounce: false,
                bounced: false,
                src: MsgAddress::Ext(MsgAddressExt::None),
                dest: MsgAddressInt::std(std_address(0x22)),
                value: CurrencyCollection::grams(Grams::from(7)),
                extra_flags: BigUint::from(2u8),
                fwd_fee: Grams::from(3),
                created_lt: 4,
                created_at: 5,
            },
            init: None,
            body: Either::Right(cell_with_bits(&[0xAD, 0x80], 9)),
        }
    }

    fn storage_phase() -> TrStoragePhase {
        TrStoragePhase {
            storage_fees_collected: Grams::from(7),
            storage_fees_due: Some(Grams::from(8)),
            status_change: AccStatusChange::Frozen,
        }
    }

    fn credit_phase() -> TrCreditPhase {
        TrCreditPhase {
            due_fees_collected: Some(Grams::from(1)),
            credit: CurrencyCollection::grams(Grams::from(10)),
        }
    }

    fn compute_skipped() -> TrComputePhase {
        TrComputePhase::Skipped {
            reason: ComputeSkipReason::NoGas,
        }
    }

    fn compute_vm() -> TrComputePhase {
        TrComputePhase::Vm {
            success: true,
            msg_state_used: false,
            account_activated: true,
            gas_fees: Grams::from(11),
            gas_used: BigUint::from(12u8),
            gas_limit: BigUint::from(13u8),
            gas_credit: Some(BigUint::from(2u8)),
            mode: -1,
            exit_code: -14,
            exit_arg: Some(32),
            vm_steps: 1234,
            vm_init_state_hash: [0x11; 32],
            vm_final_state_hash: [0x22; 32],
        }
    }

    fn action_phase() -> TrActionPhase {
        TrActionPhase {
            success: true,
            valid: true,
            no_funds: false,
            status_change: AccStatusChange::Unchanged,
            total_fwd_fees: Some(Grams::from(3)),
            total_action_fees: None,
            result_code: 0,
            result_arg: None,
            tot_actions: 1,
            spec_actions: 0,
            skipped_actions: 0,
            msgs_created: 1,
            action_list_hash: [0x33; 32],
            tot_msg_size: StorageUsed::new(BigUint::from(1u8), BigUint::from(64u8)),
        }
    }

    fn split_info() -> SplitMergeInfo {
        SplitMergeInfo {
            cur_shard_pfx_len: 12,
            acc_split_depth: 6,
            this_addr: [0x44; 32],
            sibling_addr: [0x55; 32],
        }
    }

    fn hash_update() -> HashUpdateAccount {
        HashUpdateAccount {
            old_hash: [0xAA; 32],
            new_hash: [0xBB; 32],
        }
    }

    fn storage_info() -> StorageInfo {
        StorageInfo {
            used: StorageUsed::new(BigUint::from(2u8), BigUint::from(128u16)),
            last_paid: 1_700_000_001,
            due_payment: Some(Grams::from(4)),
            extra: StorageExtraInfo::Info {
                dict_hash: [0xCC; 32],
            },
        }
    }

    fn account_storage() -> AccountStorage {
        AccountStorage {
            last_trans_lt: 11,
            balance: CurrencyCollection::grams(Grams::from(100)),
            state: AccountState::Active {
                state_init: StateInit::empty(),
            },
        }
    }

    fn account_fixture_value() -> Account {
        Account::Full {
            addr: account_address(),
            storage_stat: storage_info(),
            storage: account_storage(),
        }
    }

    fn transaction_fixture_value() -> Transaction {
        Transaction {
            account_addr: [0x10; 32],
            lt: 7,
            prev_trans_hash: [0x20; 32],
            prev_trans_lt: 6,
            now: 1_700_000_000,
            outmsg_cnt: 0,
            orig_status: AccountStatus::Active,
            end_status: AccountStatus::Active,
            in_msg: None,
            out_msgs: HashmapE::new(15),
            total_fees: CurrencyCollection::grams(Grams::from(3)),
            state_update: hash_update(),
            description: TransactionDescr::Storage {
                storage_ph: storage_phase(),
            },
        }
    }

    fn simple_transaction() -> Transaction {
        transaction_fixture_value()
    }

    #[test]
    fn phase1_account_message_transaction_fixtures_are_checked() {
        let set = fixture_set(include_str!(
            "../../../fixtures/phase1/account_message_transaction.json"
        ));
        assert_fixture(
            find(&set, "message-with-referenced-state-init-and-body"),
            "Message Any",
            message_fixture_value(),
        );
        assert_fixture(
            find(&set, "relaxed-message-with-referenced-body"),
            "MessageRelaxed Any",
            relaxed_message_fixture_value(),
        );
        assert_fixture(
            find(&set, "storage-only-transaction"),
            "Transaction",
            transaction_fixture_value(),
        );
        assert_fixture(
            find(&set, "full-active-account"),
            "Account",
            account_fixture_value(),
        );
    }

    #[test]
    fn phase1_transaction_description_fixtures_cover_all_exit_cases() {
        let set = fixture_set(include_str!(
            "../../../fixtures/phase1/transaction_descriptions.json"
        ));
        assert_fixture(
            find(&set, "ordinary-transaction-description"),
            "TransactionDescr::Ordinary",
            TransactionDescr::Ordinary {
                credit_first: true,
                storage_ph: Some(storage_phase()),
                credit_ph: Some(credit_phase()),
                compute_ph: compute_skipped(),
                action: None,
                aborted: false,
                bounce: Some(TrBouncePhase::NegativeFunds),
                destroyed: false,
            },
        );
        assert_fixture(
            find(&set, "tick-tock-transaction-description"),
            "TransactionDescr::TickTock",
            TransactionDescr::TickTock {
                is_tock: true,
                storage_ph: storage_phase(),
                compute_ph: compute_vm(),
                action: Some(action_phase()),
                aborted: false,
                destroyed: true,
            },
        );
        assert_fixture(
            find(&set, "split-prepare-transaction-description"),
            "TransactionDescr::SplitPrepare",
            TransactionDescr::SplitPrepare {
                split_info: split_info(),
                storage_ph: Some(storage_phase()),
                compute_ph: compute_skipped(),
                action: None,
                aborted: true,
                destroyed: false,
            },
        );
        assert_fixture(
            find(&set, "split-install-transaction-description"),
            "TransactionDescr::SplitInstall",
            TransactionDescr::SplitInstall {
                split_info: split_info(),
                prepare_transaction: Box::new(simple_transaction()),
                installed: true,
            },
        );
        assert_fixture(
            find(&set, "merge-prepare-transaction-description"),
            "TransactionDescr::MergePrepare",
            TransactionDescr::MergePrepare {
                split_info: split_info(),
                storage_ph: storage_phase(),
                aborted: true,
            },
        );
        assert_fixture(
            find(&set, "merge-install-transaction-description"),
            "TransactionDescr::MergeInstall",
            TransactionDescr::MergeInstall {
                split_info: split_info(),
                prepare_transaction: Box::new(simple_transaction()),
                storage_ph: None,
                credit_ph: Some(credit_phase()),
                compute_ph: compute_vm(),
                action: Some(action_phase()),
                aborted: false,
                destroyed: true,
            },
        );
    }
}
