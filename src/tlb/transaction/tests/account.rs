use super::*;

use crate::tlb::{CommonMsgInfo, Either, TlbSerialize, store_tag};
use crate::tvm::{Address, BitKey, Cell, HashmapAug, HashmapAugE, HashmapAugLeaf};
use std::sync::Arc;

pub(super) fn roundtrip<T>(value: &T) -> T
where
    T: TlbSerialize + TlbDeserialize + PartialEq + std::fmt::Debug,
{
    T::from_cell(value.to_cell().unwrap()).unwrap()
}

pub(super) fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
    let mut builder = Builder::new();
    builder.store_bits(data, bit_len).unwrap();
    builder.build().unwrap()
}

pub(super) fn storage_phase() -> TrStoragePhase {
    TrStoragePhase {
        storage_fees_collected: Grams::from(7),
        storage_fees_due: Some(Grams::from(8)),
        status_change: AccStatusChange::Frozen,
    }
}

pub(super) fn credit_phase() -> TrCreditPhase {
    TrCreditPhase {
        due_fees_collected: Some(Grams::from(1)),
        credit: CurrencyCollection::grams(Grams::from(10)),
    }
}

pub(super) fn compute_skipped() -> TrComputePhase {
    TrComputePhase::Skipped {
        reason: ComputeSkipReason::NoGas,
    }
}

pub(super) fn compute_vm() -> TrComputePhase {
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

pub(super) fn action_phase() -> TrActionPhase {
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

pub(super) fn split_info() -> SplitMergeInfo {
    SplitMergeInfo {
        cur_shard_pfx_len: 12,
        acc_split_depth: 6,
        this_addr: [0x44; 32],
        sibling_addr: [0x55; 32],
    }
}

pub(super) fn account_address() -> MsgAddressInt {
    MsgAddressInt::std(Address::new(0, [0x11; 32]))
}

pub(super) fn message() -> Message {
    Message {
        info: CommonMsgInfo::ExternalIn {
            src: crate::tlb::MsgAddressExt::None,
            dest: account_address(),
            import_fee: Grams::from(1),
        },
        init: None,
        body: Either::Right(cell_with_bits(&[0x80], 1)),
    }
}

pub(super) fn hash_update() -> HashUpdateAccount {
    HashUpdateAccount {
        old_hash: [0xAA; 32],
        new_hash: [0xBB; 32],
    }
}

pub(super) fn storage_info() -> StorageInfo {
    StorageInfo {
        used: StorageUsed::new(BigUint::from(2u8), BigUint::from(128u16)),
        last_paid: 1_700_000_001,
        due_payment: Some(Grams::from(4)),
        extra: StorageExtraInfo::Info {
            dict_hash: [0xCC; 32],
        },
    }
}

pub(super) fn account_storage() -> AccountStorage {
    AccountStorage {
        last_trans_lt: 11,
        balance: CurrencyCollection::grams(Grams::from(100)),
        state: AccountState::Active {
            state_init: StateInit::empty(),
        },
    }
}

pub(super) fn account() -> Account {
    Account::Full {
        addr: account_address(),
        storage_stat: storage_info(),
        storage: account_storage(),
    }
}

pub(super) fn simple_transaction() -> Transaction {
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
        out_msgs: HashmapE::new(OUT_MSG_KEY_BITS),
        total_fees: CurrencyCollection::grams(Grams::from(3)),
        state_update: hash_update(),
        description: TransactionDescr::Storage {
            storage_ph: storage_phase(),
        },
    }
}

pub(super) fn transaction_with_messages() -> Transaction {
    let mut out_msgs = HashmapE::new(OUT_MSG_KEY_BITS);
    out_msgs
        .insert_bit_key(BitKey::from_u64(3, OUT_MSG_KEY_BITS).unwrap(), message())
        .unwrap();
    Transaction {
        in_msg: Some(message()),
        outmsg_cnt: 1,
        out_msgs,
        ..simple_transaction()
    }
}

pub(super) fn depth_balance(split_depth: u8, grams: u64) -> DepthBalanceInfo {
    DepthBalanceInfo {
        split_depth,
        balance: CurrencyCollection::grams(Grams::from(grams)),
    }
}

pub(super) fn account_block_with_lts(lts: &[u64]) -> AccountBlock {
    let entries = lts
        .iter()
        .copied()
        .map(|lt| HashmapAugLeaf {
            key: BitKey::from_u64(lt, 64).unwrap(),
            value: Transaction {
                lt,
                ..simple_transaction()
            },
            extra: CurrencyCollection::grams(Grams::from(lt + 1)),
        })
        .collect();
    AccountBlock {
        account_addr: [0x55; 32],
        transactions: HashmapAug::from_entries(
            64,
            entries,
            CurrencyCollection::grams(Grams::from(99)),
        )
        .unwrap(),
        state_update: hash_update(),
    }
}

pub(super) fn store_transaction_prefix(builder: &mut Builder) {
    store_tag(builder, "0111").unwrap();
    builder.store_bytes(&[0x10; 32]).unwrap();
    builder.store_u64(7).unwrap();
    builder.store_bytes(&[0x20; 32]).unwrap();
    builder.store_u64(6).unwrap();
    builder.store_u32(1_700_000_000).unwrap();
    builder
        .store_uint_custom::<u16>(0, OUT_MSG_KEY_BITS)
        .unwrap();
    AccountStatus::Active.store_tlb(builder).unwrap();
    AccountStatus::Active.store_tlb(builder).unwrap();
}

pub(super) fn store_transaction_suffix(builder: &mut Builder) {
    CurrencyCollection::grams(Grams::from(3))
        .store_tlb(builder)
        .unwrap();
    store_ref_tlb(builder, &hash_update()).unwrap();
    store_ref_tlb(
        builder,
        &TransactionDescr::Storage {
            storage_ph: storage_phase(),
        },
    )
    .unwrap();
}

#[test]
pub(super) fn storage_and_credit_phases_roundtrip() {
    assert_eq!(roundtrip(&storage_phase()), storage_phase());
    assert_eq!(roundtrip(&credit_phase()), credit_phase());
}

#[test]
pub(super) fn storage_extra_info_roundtrips() {
    assert_eq!(roundtrip(&StorageExtraInfo::None), StorageExtraInfo::None);
    let value = StorageExtraInfo::Info {
        dict_hash: [0x12; 32],
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
pub(super) fn storage_extra_info_and_storage_info_match_upstream_layout() {
    let none = StorageExtraInfo::None.to_cell().unwrap();
    assert_eq!(none.bit_len(), 3);
    assert_eq!(none.data()[0] >> 5, 0);

    let info = StorageExtraInfo::Info {
        dict_hash: [0x12; 32],
    }
    .to_cell()
    .unwrap();
    assert_eq!(info.bit_len(), 259);
    assert_eq!(info.data()[0] >> 5, 1);

    let value = storage_info();
    let mut slice = Slice::new(value.to_cell().unwrap());
    assert_eq!(StorageUsed::load_tlb(&mut slice).unwrap(), value.used);
    assert_eq!(slice.load_bits(3).unwrap(), vec![0b0010_0000]);
    let mut dict_hash = [0; 32];
    dict_hash.copy_from_slice(&slice.load_bytes(32).unwrap());
    assert_eq!(dict_hash, [0xCC; 32]);
    assert_eq!(slice.load_u32().unwrap(), value.last_paid);
    assert_eq!(load_maybe::<Grams>(&mut slice).unwrap(), value.due_payment);
}

#[test]
pub(super) fn account_state_all_tags_roundtrip() {
    let values = [
        AccountState::Uninit,
        AccountState::Frozen {
            state_hash: [0x34; 32],
        },
        AccountState::Active {
            state_init: StateInit::empty(),
        },
    ];
    for value in values {
        assert_eq!(roundtrip(&value), value);
    }
}

#[test]
pub(super) fn account_status_all_tags_roundtrip() {
    for value in [
        AccountStatus::Uninit,
        AccountStatus::Frozen,
        AccountStatus::Active,
        AccountStatus::Nonexist,
    ] {
        assert_eq!(roundtrip(&value), value);
    }
}

#[test]
pub(super) fn account_and_shard_account_roundtrip() {
    assert_eq!(roundtrip(&Account::None), Account::None);
    assert_eq!(roundtrip(&account()), account());

    let shard = ShardAccount {
        account: account(),
        last_trans_hash: [0x44; 32],
        last_trans_lt: 12,
    };
    assert_eq!(roundtrip(&shard), shard);
}

#[test]
pub(super) fn depth_balance_and_augmented_account_collections_roundtrip() {
    assert_eq!(roundtrip(&depth_balance(30, 123)), depth_balance(30, 123));

    let shard_account = ShardAccount {
        account: account(),
        last_trans_hash: [0x44; 32],
        last_trans_lt: 12,
    };
    let shard_root = HashmapAug::from_entries(
        256,
        vec![HashmapAugLeaf {
            key: BitKey::from_bits(vec![0x11; 32], 256).unwrap(),
            value: shard_account,
            extra: depth_balance(7, 100),
        }],
        depth_balance(7, 100),
    )
    .unwrap();
    let shard_accounts = ShardAccounts {
        accounts: HashmapAugE::with_root(256, shard_root, depth_balance(7, 100)).unwrap(),
    };
    assert_eq!(roundtrip(&shard_accounts), shard_accounts);

    let blocks_root = HashmapAug::from_entries(
        256,
        vec![HashmapAugLeaf {
            key: BitKey::from_bits(vec![0x22; 32], 256).unwrap(),
            value: account_block_with_lts(&[7]),
            extra: CurrencyCollection::grams(Grams::from(8)),
        }],
        CurrencyCollection::grams(Grams::from(8)),
    )
    .unwrap();
    let blocks = ShardAccountBlocks {
        blocks: HashmapAugE::with_root(256, blocks_root, CurrencyCollection::grams(Grams::from(8)))
            .unwrap(),
    };
    assert_eq!(roundtrip(&blocks), blocks);
}

#[test]
pub(super) fn account_block_roundtrips_with_one_and_multiple_transactions() {
    assert_eq!(
        roundtrip(&account_block_with_lts(&[7])),
        account_block_with_lts(&[7])
    );
    assert_eq!(
        roundtrip(&account_block_with_lts(&[7, 9])),
        account_block_with_lts(&[7, 9])
    );
}

#[test]
pub(super) fn depth_balance_rejects_split_depth_above_30() {
    let err = depth_balance(31, 1).to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "DepthBalanceInfo.split_depth",
            ..
        }
    ));

    let mut builder = Builder::new();
    builder.store_uint_custom::<u8>(31, 5).unwrap();
    CurrencyCollection::grams(Grams::from(1))
        .store_tlb(&mut builder)
        .unwrap();
    let err = DepthBalanceInfo::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::NonCanonicalValue {
            schema: "DepthBalanceInfo.split_depth",
            ..
        }
    ));
}

#[test]
pub(super) fn account_block_reports_malformed_references() {
    let bad_ref_entries = vec![HashmapAugLeaf {
        key: BitKey::from_u64(7, 64).unwrap(),
        value: cell_with_bits(&[0x80], 1),
        extra: CurrencyCollection::grams(Grams::from(1)),
    }];
    let bad_ref_dict = HashmapAug::from_entries(
        64,
        bad_ref_entries,
        CurrencyCollection::grams(Grams::from(1)),
    )
    .unwrap();
    let mut builder = Builder::new();
    store_tag(&mut builder, "0101").unwrap();
    builder.store_bytes(&[0x55; 32]).unwrap();
    builder
        .store_hashmap_aug_with(
            &bad_ref_dict,
            |builder, cell| {
                builder.store_ref(cell.clone())?;
                Ok(())
            },
            |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
        )
        .unwrap();
    store_ref_tlb(&mut builder, &hash_update()).unwrap();
    let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "Transaction",
            ..
        }
    ));

    let mut builder = Builder::new();
    store_tag(&mut builder, "0101").unwrap();
    builder.store_bytes(&[0x55; 32]).unwrap();
    builder
        .store_hashmap_aug_with(
            &account_block_with_lts(&[7]).transactions,
            |builder, transaction| store_ref_tlb(builder, transaction).map_err(anyhow::Error::from),
            |builder, extra| extra.store_tlb(builder).map_err(anyhow::Error::from),
        )
        .unwrap();
    builder.store_ref(cell_with_bits(&[0x71], 8)).unwrap();
    let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "HASH_UPDATE Account",
            ..
        }
    ));
}

#[test]
pub(super) fn exact_account_block_decode_rejects_trailing_data() {
    let mut builder = Builder::new();
    account_block_with_lts(&[7])
        .store_tlb(&mut builder)
        .unwrap();
    builder.store_bit(true).unwrap();
    let err = AccountBlock::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
}

#[test]
pub(super) fn hash_update_account_roundtrips() {
    assert_eq!(roundtrip(&hash_update()), hash_update());
}

#[test]
pub(super) fn transaction_roundtrips_without_and_with_messages() {
    assert_eq!(roundtrip(&simple_transaction()), simple_transaction());
    assert_eq!(
        roundtrip(&transaction_with_messages()),
        transaction_with_messages()
    );
}

#[test]
pub(super) fn transaction_serialization_rejects_outmsg_count_above_uint15() {
    let value = Transaction {
        outmsg_cnt: 0x8000,
        ..simple_transaction()
    };
    let err = value.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "Transaction.outmsg_cnt",
            ..
        }
    ));
}
