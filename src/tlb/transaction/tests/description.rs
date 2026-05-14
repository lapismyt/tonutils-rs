use super::account::*;
use super::*;
use crate::tvm::BitKey;

#[test]
fn transaction_serialization_rejects_non_15_bit_out_msg_dictionary() {
    let value = Transaction {
        out_msgs: HashmapE::new(16),
        ..simple_transaction()
    };
    let err = value.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "Transaction.out_msgs",
            ..
        }
    ));
}

#[test]
fn truncated_new_constructor_tags_are_rejected() {
    let err = Account::from_cell(Builder::new().build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::Tvm(_)));

    let err = AccountState::from_cell(cell_with_bits(&[0], 1)).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "AccountState",
            actual_bits,
            ..
        } if actual_bits == "0"
    ));

    let err = AccountStatus::from_cell(Builder::new().build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "AccountStatus",
            actual_bits,
            ..
        } if actual_bits.is_empty()
    ));

    let err = HashUpdateAccount::from_cell(cell_with_bits(&[0x71], 8)).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "HASH_UPDATE Account",
            ..
        }
    ));

    let err = HashUpdateAccount::from_cell(cell_with_bits(&[0x70], 4)).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "HASH_UPDATE Account",
            actual_bits,
            ..
        } if actual_bits == "0111"
    ));

    let err = Transaction::from_cell(cell_with_bits(&[0x80], 1)).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "Transaction",
            actual_bits,
            ..
        } if actual_bits == "1"
    ));
}

#[test]
fn malformed_transaction_message_reference_is_reported() {
    let mut messages = Builder::new();
    messages.store_bit(true).unwrap();
    messages.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
    messages.store_bit(false).unwrap();

    let mut builder = Builder::new();
    store_transaction_prefix(&mut builder);
    builder.store_ref(messages.build().unwrap()).unwrap();
    store_transaction_suffix(&mut builder);

    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "Message Any",
            ..
        }
    ));
}

#[test]
fn malformed_transaction_out_message_reference_is_reported() {
    let mut out_msgs = HashmapE::new(OUT_MSG_KEY_BITS);
    out_msgs
        .insert_bit_key(
            BitKey::from_u64(1, OUT_MSG_KEY_BITS).unwrap(),
            cell_with_bits(&[0x80], 1),
        )
        .unwrap();

    let mut messages = Builder::new();
    messages.store_bit(false).unwrap();
    messages
        .store_hashmap_e_with(&out_msgs, |builder, cell| {
            builder.store_ref(cell.clone())?;
            Ok(())
        })
        .unwrap();

    let mut builder = Builder::new();
    store_transaction_prefix(&mut builder);
    builder.store_ref(messages.build().unwrap()).unwrap();
    store_transaction_suffix(&mut builder);

    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "Message Any",
            ..
        }
    ));
}

#[test]
fn malformed_transaction_state_update_and_description_references_are_reported() {
    let mut messages = Builder::new();
    messages.store_bit(false).unwrap();
    messages.store_bit(false).unwrap();

    let mut builder = Builder::new();
    store_transaction_prefix(&mut builder);
    builder.store_ref(messages.build().unwrap()).unwrap();
    CurrencyCollection::grams(Grams::from(3))
        .store_tlb(&mut builder)
        .unwrap();
    builder.store_ref(cell_with_bits(&[0x71], 8)).unwrap();
    store_ref_tlb(
        &mut builder,
        &TransactionDescr::Storage {
            storage_ph: storage_phase(),
        },
    )
    .unwrap();
    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "HASH_UPDATE Account",
            ..
        }
    ));

    let mut messages = Builder::new();
    messages.store_bit(false).unwrap();
    messages.store_bit(false).unwrap();
    let mut builder = Builder::new();
    store_transaction_prefix(&mut builder);
    builder.store_ref(messages.build().unwrap()).unwrap();
    CurrencyCollection::grams(Grams::from(3))
        .store_tlb(&mut builder)
        .unwrap();
    store_ref_tlb(&mut builder, &hash_update()).unwrap();
    builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "TransactionDescr",
            ..
        }
    ));
}

#[test]
fn exact_transaction_decode_rejects_trailing_data() {
    let mut builder = Builder::new();
    simple_transaction().store_tlb(&mut builder).unwrap();
    builder.store_bit(true).unwrap();
    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

    let mut builder = Builder::new();
    simple_transaction().store_tlb(&mut builder).unwrap();
    builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();
    let err = Transaction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
}

#[test]
fn compute_skip_reason_all_tags_roundtrip() {
    for reason in [
        ComputeSkipReason::NoState,
        ComputeSkipReason::BadState,
        ComputeSkipReason::NoGas,
        ComputeSkipReason::Suspended,
    ] {
        assert_eq!(roundtrip(&reason), reason);
    }
}

#[test]
fn compute_phases_roundtrip() {
    assert_eq!(roundtrip(&compute_skipped()), compute_skipped());
    assert_eq!(roundtrip(&compute_vm()), compute_vm());
}

#[test]
fn bounce_phase_all_tags_roundtrip() {
    let msg_size = StorageUsed::new(BigUint::from(2u8), BigUint::from(16u8));
    let values = [
        TrBouncePhase::NegativeFunds,
        TrBouncePhase::NoFunds {
            msg_size: msg_size.clone(),
            req_fwd_fees: Grams::from(9),
        },
        TrBouncePhase::Ok {
            msg_size,
            msg_fees: Grams::from(10),
            fwd_fees: Grams::from(11),
        },
    ];
    for value in values {
        assert_eq!(roundtrip(&value), value);
    }
}

#[test]
fn split_merge_info_roundtrips_and_rejects_out_of_range_values() {
    assert_eq!(roundtrip(&split_info()), split_info());

    let invalid = SplitMergeInfo {
        cur_shard_pfx_len: 64,
        ..split_info()
    };
    let err = invalid.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "SplitMergeInfo.cur_shard_pfx_len",
            ..
        }
    ));
}

#[test]
fn ordinary_description_roundtrips_without_action() {
    let value = TransactionDescr::Ordinary {
        credit_first: true,
        storage_ph: Some(storage_phase()),
        credit_ph: Some(credit_phase()),
        compute_ph: compute_skipped(),
        action: None,
        aborted: false,
        bounce: Some(TrBouncePhase::NegativeFunds),
        destroyed: false,
    };

    assert_eq!(roundtrip(&value), value);
}

#[test]
fn ordinary_description_roundtrips_with_referenced_action() {
    let value = TransactionDescr::Ordinary {
        credit_first: false,
        storage_ph: None,
        credit_ph: Some(credit_phase()),
        compute_ph: compute_vm(),
        action: Some(action_phase()),
        aborted: true,
        bounce: Some(TrBouncePhase::Ok {
            msg_size: StorageUsed::new(BigUint::from(1u8), BigUint::from(1u8)),
            msg_fees: Grams::from(2),
            fwd_fees: Grams::from(3),
        }),
        destroyed: true,
    };

    assert_eq!(roundtrip(&value), value);
}

#[test]
fn storage_only_description_roundtrips() {
    let value = TransactionDescr::Storage {
        storage_ph: storage_phase(),
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
fn tick_tock_description_roundtrips() {
    let value = TransactionDescr::TickTock {
        is_tock: true,
        storage_ph: storage_phase(),
        compute_ph: compute_vm(),
        action: Some(action_phase()),
        aborted: false,
        destroyed: true,
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
fn split_prepare_description_roundtrips() {
    let value = TransactionDescr::SplitPrepare {
        split_info: split_info(),
        storage_ph: Some(storage_phase()),
        compute_ph: compute_skipped(),
        action: None,
        aborted: true,
        destroyed: false,
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
fn split_install_description_roundtrips_with_typed_prepare_transaction() {
    let prepare_transaction = Box::new(simple_transaction());
    let value = TransactionDescr::SplitInstall {
        split_info: split_info(),
        prepare_transaction: prepare_transaction.clone(),
        installed: true,
    };

    let decoded = roundtrip(&value);
    assert_eq!(decoded, value);
    match decoded {
        TransactionDescr::SplitInstall {
            prepare_transaction: decoded,
            ..
        } => assert_eq!(decoded, prepare_transaction),
        _ => panic!("expected split install"),
    }
}

#[test]
fn merge_prepare_description_roundtrips() {
    let value = TransactionDescr::MergePrepare {
        split_info: split_info(),
        storage_ph: storage_phase(),
        aborted: true,
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
fn merge_install_description_roundtrips_with_typed_prepare_transaction() {
    let prepare_transaction = Box::new(simple_transaction());
    let value = TransactionDescr::MergeInstall {
        split_info: split_info(),
        prepare_transaction: prepare_transaction.clone(),
        storage_ph: None,
        credit_ph: Some(credit_phase()),
        compute_ph: compute_vm(),
        action: Some(action_phase()),
        aborted: false,
        destroyed: true,
    };

    let decoded = roundtrip(&value);
    assert_eq!(decoded, value);
    match decoded {
        TransactionDescr::MergeInstall {
            prepare_transaction: decoded,
            ..
        } => assert_eq!(decoded, prepare_transaction),
        _ => panic!("expected merge install"),
    }
}

#[test]
fn unknown_and_truncated_transaction_description_tags_are_rejected() {
    let mut builder = Builder::new();
    store_tag(&mut builder, "1").unwrap();
    let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "TransactionDescr",
            actual_bits,
            ..
        } if actual_bits == "1"
    ));

    let mut builder = Builder::new();
    store_tag(&mut builder, "000").unwrap();
    let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "TransactionDescr",
            actual_bits,
            ..
        } if actual_bits == "000"
    ));
}

#[test]
fn invalid_and_truncated_compute_skip_reason_tags_are_rejected() {
    let mut builder = Builder::new();
    store_tag(&mut builder, "111").unwrap();
    let err = ComputeSkipReason::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "ComputeSkipReason",
            actual_bits,
            ..
        } if actual_bits == "111"
    ));

    let mut builder = Builder::new();
    store_tag(&mut builder, "11").unwrap();
    let err = ComputeSkipReason::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "ComputeSkipReason",
            actual_bits,
            ..
        } if actual_bits == "11"
    ));
}

#[test]
fn malformed_referenced_action_phase_payload_is_reported() {
    let mut invalid_action = Builder::new();
    invalid_action.store_bit(true).unwrap();

    let mut builder = Builder::new();
    store_tag(&mut builder, "0000").unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();
    compute_skipped().store_tlb(&mut builder).unwrap();
    builder.store_bit(true).unwrap();
    builder.store_ref(invalid_action.build().unwrap()).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();

    let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "TrActionPhase",
            ..
        }
    ));
}

#[test]
fn compute_vm_malformed_referenced_payload_is_reported() {
    let mut builder = Builder::new();
    store_tag(&mut builder, "1").unwrap();
    builder.store_bit(true).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();
    Grams::from(1).store_tlb(&mut builder).unwrap();
    builder.store_ref(cell_with_bits(&[0x80], 1)).unwrap();

    let err = TrComputePhase::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "TrComputePhase.vm",
            ..
        }
    ));
}

#[test]
fn exact_transaction_description_decode_rejects_trailing_data() {
    let value = TransactionDescr::Storage {
        storage_ph: storage_phase(),
    };
    let mut builder = Builder::new();
    value.store_tlb(&mut builder).unwrap();
    builder.store_bit(true).unwrap();
    let err = TransactionDescr::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
}
