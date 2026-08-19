use super::*;
use crate::{DepthBalanceInfo, TlbDeserialize, TlbSerialize};
use tonutils_tvm::Builder;

#[test]
fn shard_ident_roundtrips_and_checks_bound() {
    let ident = ShardIdent {
        shard_pfx_bits: 60,
        workchain_id: -1,
        shard_prefix: 0x8000_0000_0000_0000,
    };
    let cell = ident.to_cell().unwrap();
    assert_eq!(ShardIdent::from_cell(cell).unwrap(), ident);

    let invalid = ShardIdent {
        shard_pfx_bits: 61,
        workchain_id: 0,
        shard_prefix: 0,
    };
    assert!(invalid.to_cell().is_err());
}

#[test]
fn block_wrapper_roundtrips_referenced_children() {
    let child = Builder::new().build().unwrap();
    let block = Block {
        global_id: -239,
        info: child.clone(),
        value_flow: child.clone(),
        state_update: child.clone(),
        extra: child,
    };

    let cell = block.to_cell().unwrap();
    let decoded = Block::from_cell(cell.clone()).unwrap();
    assert_eq!(decoded, block);
    assert_eq!(decoded.to_cell().unwrap().hash(), cell.hash());
}

#[test]
fn value_flow_rejects_unknown_constructor() {
    let mut builder = Builder::new();
    builder.store_u32(0xfeed_beef).unwrap();
    let err = ValueFlow::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TagMismatch { .. }));
}

#[test]
fn block_info_roundtrips_conditional_fields() {
    let previous = ExtBlkRef {
        end_lt: 11,
        seq_no: 12,
        root_hash: [0x11; 32],
        file_hash: [0x22; 32],
    };
    let info = BlockInfo {
        version: 7,
        not_master: true,
        after_merge: false,
        before_split: true,
        after_split: false,
        want_split: true,
        want_merge: false,
        key_block: true,
        vert_seqno_incr: true,
        flags: 0,
        seq_no: 13,
        vert_seq_no: 14,
        shard: ShardIdent {
            shard_pfx_bits: 60,
            workchain_id: 0,
            shard_prefix: 0x8000_0000_0000_0000,
        },
        gen_utime: 15,
        start_lt: 16,
        end_lt: 17,
        gen_validator_list_hash_short: 18,
        gen_catchain_seqno: 19,
        min_ref_mc_seqno: 20,
        prev_key_block_seqno: 21,
        gen_software: Some(GlobalVersion {
            version: 22,
            capabilities: 23,
        }),
        master_ref: Some(BlkMasterInfo {
            master: previous.clone(),
        }),
        prev_ref: BlockPrevInfo::Single {
            prev: previous.clone(),
        },
        prev_vert_ref: Some(previous),
    };

    let cell = info.to_cell().unwrap();
    assert_eq!(BlockInfo::from_cell(cell.clone()).unwrap(), info);
    assert_eq!(
        BlockInfo::from_cell(cell.clone())
            .unwrap()
            .to_cell()
            .unwrap(),
        cell
    );
}

#[test]
fn value_flow_v1_and_v2_roundtrip_typed_currency_groups() {
    let collection = |amount: u64| CurrencyCollection::grams(amount.into());
    let main = ValueFlowMain {
        from_prev_blk: collection(1),
        to_next_blk: collection(2),
        imported: collection(3),
        exported: collection(4),
    };
    let fees = ValueFlowFees {
        fees_imported: collection(5),
        recovered: collection(6),
        created: collection(7),
        minted: collection(8),
    };
    let v1 = ValueFlow::V1 {
        main: main.clone(),
        fees_collected: collection(9),
        fees: fees.clone(),
    };
    let v2 = ValueFlow::V2 {
        main,
        fees_collected: collection(10),
        burned: collection(11),
        fees,
    };

    for value in [v1, v2] {
        let cell = value.to_cell().unwrap();
        assert_eq!(ValueFlow::from_cell(cell.clone()).unwrap(), value);
        assert_eq!(
            ValueFlow::from_cell(cell.clone())
                .unwrap()
                .to_cell()
                .unwrap(),
            cell
        );
    }
}

#[test]
fn hash_update_uses_eight_bit_constructor_tag() {
    let update = HashUpdate {
        old_hash: [0x11; 32],
        new_hash: [0x22; 32],
    };

    let cell = update.to_cell().unwrap();
    assert_eq!(cell.bit_len(), 8 + 256 + 256);
    assert_eq!(HashUpdate::from_cell(cell).unwrap(), update);
}

#[test]
fn shard_state_unsplit_decodes_stable_fields_and_preserves_raw_children() {
    let empty = Builder::new().build().unwrap();
    let state = ShardStateUnsplitData {
        global_id: -239,
        shard_id: ShardIdent {
            shard_pfx_bits: 60,
            workchain_id: 0,
            shard_prefix: 0x8000_0000_0000_0000,
        },
        seq_no: 1,
        vert_seq_no: 2,
        gen_utime: 1_700_000_000,
        gen_lt: 3,
        min_ref_mc_seqno: 4,
        out_msg_queue_info: empty.clone(),
        before_split: true,
        accounts: ShardAccounts {
            accounts: tonutils_tvm::HashmapAugE::empty(
                256,
                DepthBalanceInfo {
                    split_depth: 0,
                    balance: CurrencyCollection::grams(0u64.into()),
                },
            ),
        },
        overload_history: 5,
        underload_history: 6,
        total_balance: CurrencyCollection::grams(7u64.into()),
        total_validator_fees: CurrencyCollection::grams(8u64.into()),
        libraries: empty,
        master_ref: None,
        custom: None,
    };

    let cell = state.to_cell().unwrap();
    assert_eq!(
        ShardStateUnsplitData::from_cell(cell.clone()).unwrap(),
        state
    );
    let wrapper = ShardStateUnsplit { cell };
    assert_eq!(wrapper.decode_typed().unwrap(), state);
}
