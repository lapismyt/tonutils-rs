use super::*;
use crate::{DepthBalanceInfo, TlbDeserialize, TlbSerialize};
use std::sync::Arc;
use tonutils_tvm::{Builder, Cell, ExoticCellKind, boc_to_hex};

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

#[test]
fn known_config_payloads_roundtrip_and_decode_typed() {
    let mut election_builder = Builder::new();
    election_builder.store_u32(1).unwrap();
    election_builder.store_u32(2).unwrap();
    election_builder.store_u32(3).unwrap();
    election_builder.store_u32(4).unwrap();
    let election_cell = election_builder.build().unwrap();
    let election = ConfigParam {
        id: 15,
        value: ConfigParamValue::Param15(election_cell),
    };
    assert!(matches!(
        election.decode_typed().unwrap(),
        Some(ConfigParamPayload::ValidatorElection {
            validators_elected_for: 1,
            elections_start_before: 2,
            elections_end_before: 3,
            stake_held_for: 4,
        })
    ));

    let gas = GasLimitsPrices::FlatPrefix {
        flat_gas_limit: 10,
        flat_gas_price: 11,
        other: Box::new(GasLimitsPrices::Extended {
            gas_price: 12,
            gas_limit: 13,
            special_gas_limit: 14,
            gas_credit: 15,
            block_gas_limit: 16,
            freeze_due_limit: 17,
            delete_due_limit: 18,
        }),
    };
    let gas_param = ConfigParam {
        id: 20,
        value: ConfigParamValue::Param20(gas.to_cell().unwrap()),
    };
    assert_eq!(
        gas_param.decode_typed().unwrap(),
        Some(ConfigParamPayload::GasLimits(gas))
    );

    let prices = MsgForwardPrices {
        lump_price: 1,
        bit_price: 2,
        cell_price: 3,
        ihr_price_factor: 4,
        first_frac: 5,
        next_frac: 6,
    };
    let price_param = ConfigParam {
        id: 24,
        value: ConfigParamValue::Param24(prices.to_cell().unwrap()),
    };
    assert_eq!(
        price_param.decode_typed().unwrap(),
        Some(ConfigParamPayload::ForwardPrices(prices))
    );
}

#[test]
fn known_config_payloads_reject_wrong_constructor_or_trailing_data() {
    let mut malformed_gas_builder = Builder::new();
    malformed_gas_builder.store_uint(0xffu8).unwrap();
    let malformed_gas = malformed_gas_builder.build().unwrap();
    let param = ConfigParam {
        id: 20,
        value: ConfigParamValue::Param20(malformed_gas),
    };
    assert!(param.decode_typed().is_err());

    let mut malformed_election_builder = Builder::new();
    malformed_election_builder.store_u32(1).unwrap();
    malformed_election_builder.store_u32(2).unwrap();
    malformed_election_builder.store_u32(3).unwrap();
    malformed_election_builder.store_u32(4).unwrap();
    malformed_election_builder.store_bit(true).unwrap();
    let malformed_election = malformed_election_builder.build().unwrap();
    let param = ConfigParam {
        id: 15,
        value: ConfigParamValue::Param15(malformed_election),
    };
    assert!(param.decode_typed().is_err());
}

#[test]
fn merkle_wrappers_validate_exotic_kind_references_and_hashes() {
    let mut root_builder = Builder::new();
    root_builder.store_bit(true).unwrap();
    let root = root_builder.build().unwrap();
    let mut proof_data = vec![0x03];
    proof_data.extend_from_slice(&root.hash());
    proof_data.extend_from_slice(&7u16.to_be_bytes());
    let proof = Arc::new(Cell::with_exotic_data(proof_data, 280, vec![root.clone()]).unwrap());
    let proof = MerkleProof::from_exotic_cell(proof).unwrap();
    assert!(proof.verify_virtual_hash());
    assert_eq!(proof.depth, 7);
    assert!(matches!(
        proof.cell.exotic_kind(),
        Some(ExoticCellKind::MerkleProof { .. })
    ));

    let mut other_builder = Builder::new();
    other_builder.store_bit(false).unwrap();
    let other = other_builder.build().unwrap();
    let mut update_data = vec![0x04];
    update_data.extend_from_slice(&root.hash());
    update_data.extend_from_slice(&other.hash());
    update_data.extend_from_slice(&1u16.to_be_bytes());
    update_data.extend_from_slice(&2u16.to_be_bytes());
    let update = Arc::new(
        Cell::with_exotic_data(update_data, 552, vec![root.clone(), other.clone()]).unwrap(),
    );
    let update = MerkleUpdate::from_exotic_cell(update).unwrap();
    assert!(update.verify_virtual_hashes());
    assert_eq!(update.old_depth, 1);
    assert_eq!(update.new_depth, 2);
    assert!(matches!(
        update.cell.exotic_kind(),
        Some(ExoticCellKind::MerkleUpdate { .. })
    ));

    let ordinary = Builder::new().build().unwrap();
    assert!(MerkleProof::from_exotic_cell(ordinary).is_err());
}

#[test]
fn merkle_fixture_boc_is_canonical() {
    let mut root_builder = Builder::new();
    root_builder.store_bit(true).unwrap();
    let root = root_builder.build().unwrap();
    let mut data = vec![0x03];
    data.extend_from_slice(&root.hash());
    data.extend_from_slice(&0u16.to_be_bytes());
    let proof = Arc::new(Cell::with_exotic_data(data, 280, vec![root]).unwrap());
    let boc = boc_to_hex(&proof, false).unwrap();
    let decoded = tonutils_tvm::hex_to_boc(&boc).unwrap();
    assert_eq!(decoded.hash(), proof.hash());
    assert_eq!(boc_to_hex(&decoded, false).unwrap(), boc);
}
