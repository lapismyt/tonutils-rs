use super::core::*;
use super::*;
use crate::tvm::BitKey;

#[test]
fn out_action_reserve_currency_roundtrips_extra_currency_dictionary() {
    let mut other = HashmapE::new(32);
    other
        .insert_bit_key(
            BitKey::from_u64(0x1234_5678, 32).unwrap(),
            BigUint::from(9_999u16),
        )
        .unwrap();
    let currency = CurrencyCollection {
        grams: Grams::from(123),
        other,
    };
    let action = OutAction::ReserveCurrency {
        mode: 255,
        currency,
    };

    assert_eq!(roundtrip(&action), action);
}

#[test]
fn out_action_change_library_roundtrips_hash_and_reference_forms() {
    let hash = [0x51; 32];
    let hash_action = OutAction::ChangeLibrary {
        mode: 127,
        libref: LibRef::Hash(hash),
    };
    assert_eq!(roundtrip(&hash_action), hash_action);

    let library = cell_with_bits(&[0xCE], 8);
    let ref_action = OutAction::ChangeLibrary {
        mode: 6,
        libref: LibRef::Ref(library.clone()),
    };
    let decoded = roundtrip(&ref_action);
    assert_eq!(decoded, ref_action);
    match decoded {
        OutAction::ChangeLibrary {
            libref: LibRef::Ref(decoded_library),
            ..
        } => assert_eq!(decoded_library.hash(), library.hash()),
        _ => panic!("expected change library reference action"),
    }
}

#[test]
fn out_action_unknown_and_truncated_tags_are_rejected() {
    let mut builder = Builder::new();
    builder.store_uint::<u32>(0xffff_ffff as u32).unwrap();
    let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "OutAction",
            actual_bits,
            ..
        } if actual_bits.len() == 32
    ));

    let mut builder = Builder::new();
    builder.store_bits(&[0x0e, 0xc0], 12).unwrap();
    let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "OutAction",
            actual_bits,
            ..
        } if actual_bits.len() == 12
    ));
}

#[test]
fn libref_truncated_tag_is_rejected() {
    let err = LibRef::from_cell(Builder::new().build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "LibRef",
            actual_bits,
            ..
        } if actual_bits.is_empty()
    ));
}

#[test]
fn change_library_mode_above_seven_bits_is_rejected() {
    let action = OutAction::ChangeLibrary {
        mode: 128,
        libref: LibRef::Hash([0; 32]),
    };
    let err = action.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "OutAction.action_change_library.mode",
            ..
        }
    ));
}

#[test]
fn send_msg_invalid_referenced_payload_reports_reference_failure() {
    let mut invalid_message = Builder::new();
    store_tag(&mut invalid_message, "10").unwrap();
    let mut builder = Builder::new();
    builder
        .store_uint::<u32>(ACTION_SEND_MSG_TAG as u32)
        .unwrap();
    builder.store_uint::<u8>(0).unwrap();
    builder.store_ref(invalid_message.build().unwrap()).unwrap();

    let err = OutAction::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::InvalidReferencePayload {
            schema: "MessageRelaxed Any",
            ..
        }
    ));
}

fn sample_send_action(mode: u8, body_byte: u8) -> OutAction {
    OutAction::SendMsg {
        mode,
        out_msg: MessageRelaxed {
            info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
            init: None,
            body: Either::Right(cell_with_bits(&[body_byte], 8)),
        },
    }
}

fn sample_set_code_action(byte: u8) -> OutAction {
    OutAction::SetCode {
        new_code: cell_with_bits(&[byte], 8),
    }
}

fn sample_reserve_action(mode: u8, grams: u64) -> OutAction {
    OutAction::ReserveCurrency {
        mode,
        currency: CurrencyCollection {
            grams: Grams::from(grams),
            other: HashmapE::new(32),
        },
    }
}

fn sample_change_library_action(mode: u8, byte: u8) -> OutAction {
    OutAction::ChangeLibrary {
        mode,
        libref: LibRef::Hash([byte; 32]),
    }
}

fn sample_action_phase() -> TrActionPhase {
    TrActionPhase {
        success: true,
        valid: true,
        no_funds: false,
        status_change: AccStatusChange::Unchanged,
        total_fwd_fees: None,
        total_action_fees: None,
        result_code: 0,
        result_arg: None,
        tot_actions: 0,
        spec_actions: 0,
        skipped_actions: 0,
        msgs_created: 0,
        action_list_hash: [0; 32],
        tot_msg_size: StorageUsed::new(BigUint::from(0u8), BigUint::from(0u8)),
    }
}

fn store_action_phase_prefix_through_hash(builder: &mut Builder) {
    builder.store_bit(true).unwrap();
    builder.store_bit(true).unwrap();
    builder.store_bit(false).unwrap();
    AccStatusChange::Unchanged.store_tlb(builder).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_int(0, 32).unwrap();
    builder.store_bit(false).unwrap();
    builder.store_uint::<u16>(0).unwrap();
    builder.store_uint::<u16>(0).unwrap();
    builder.store_uint::<u16>(0).unwrap();
    builder.store_uint::<u16>(0).unwrap();
    builder.store_bytes(&[0; 32]).unwrap();
}

fn contains_out_list_custom_schema(err: &TlbError) -> bool {
    match err {
        TlbError::CustomSchema {
            schema: "OutList", ..
        } => true,
        TlbError::InvalidReferencePayload { source, .. } => contains_out_list_custom_schema(source),
        _ => false,
    }
}

#[test]
fn out_list_empty_roundtrips() {
    let list = OutList::default();
    let cell = list.to_cell().unwrap();

    assert_eq!(cell.bit_len(), 0);
    assert_eq!(cell.reference_count(), 0);
    assert_eq!(OutList::from_cell(cell).unwrap(), list);
}

#[test]
fn out_list_single_action_roundtrips() {
    let list = OutList::new(vec![sample_send_action(1, 0xAA)]);

    assert_eq!(roundtrip(&list), list);
}

#[test]
fn out_list_multi_action_roundtrip_preserves_order() {
    let list = OutList::new(vec![
        sample_set_code_action(0x10),
        sample_send_action(2, 0x20),
        sample_change_library_action(3, 0x30),
    ]);

    let decoded = roundtrip(&list);
    assert_eq!(decoded.actions, list.actions);
}

#[test]
fn out_list_mixed_action_variants_roundtrip() {
    let list = OutList::new(vec![
        sample_send_action(4, 0x40),
        sample_set_code_action(0x50),
        sample_reserve_action(6, 7),
        sample_change_library_action(7, 0x80),
    ]);

    assert_eq!(roundtrip(&list), list);
}

#[test]
fn out_list_serialization_rejects_more_than_255_actions() {
    let list = OutList::new(
        (0..=MAX_OUT_LIST_ACTIONS)
            .map(|idx| sample_set_code_action(idx as u8))
            .collect(),
    );

    let err = list.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "OutList",
            ..
        }
    ));
}

#[test]
fn out_list_decode_rejects_more_than_255_nodes() {
    let mut current = Builder::new().build().unwrap();
    for idx in 0..=MAX_OUT_LIST_ACTIONS {
        let mut node = Builder::new();
        node.store_ref(current).unwrap();
        sample_set_code_action(idx as u8)
            .store_tlb(&mut node)
            .unwrap();
        current = node.build().unwrap();
    }

    let err = OutList::from_cell(current).unwrap_err();
    assert!(contains_out_list_custom_schema(&err));
}

#[test]
fn out_list_non_empty_node_without_previous_ref_is_rejected() {
    let mut builder = Builder::new();
    builder.store_bit(true).unwrap();

    let err = OutList::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::CustomSchema {
            schema: "OutList",
            ..
        }
    ));
}

#[test]
fn out_list_malformed_current_action_reports_action_decode_failure() {
    let mut builder = Builder::new();
    builder.store_ref(Builder::new().build().unwrap()).unwrap();
    builder.store_uint::<u32>(0xffff_ffff as u32).unwrap();

    let err = OutList::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "OutAction",
            ..
        }
    ));
}

#[test]
fn acc_status_change_variants_roundtrip() {
    assert_eq!(
        roundtrip(&AccStatusChange::Unchanged),
        AccStatusChange::Unchanged
    );
    assert_eq!(roundtrip(&AccStatusChange::Frozen), AccStatusChange::Frozen);
    assert_eq!(
        roundtrip(&AccStatusChange::Deleted),
        AccStatusChange::Deleted
    );
}

#[test]
fn acc_status_change_truncated_tags_are_rejected() {
    let err = AccStatusChange::from_cell(Builder::new().build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "AccStatusChange",
            actual_bits,
            ..
        } if actual_bits.is_empty()
    ));

    let mut builder = Builder::new();
    builder.store_bit(true).unwrap();
    let err = AccStatusChange::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "AccStatusChange",
            actual_bits,
            ..
        } if actual_bits == "1"
    ));
}

#[test]
fn storage_used_roundtrips_zero_and_non_zero_values() {
    let zero = StorageUsed::new(BigUint::from(0u8), BigUint::from(0u8));
    assert_eq!(roundtrip(&zero), zero);

    let non_zero = StorageUsed::new(BigUint::from(123u8), BigUint::from(65_535u32));
    assert_eq!(roundtrip(&non_zero), non_zero);
}

#[test]
fn storage_used_rejects_non_canonical_varuint() {
    let mut builder = Builder::new();
    builder
        .store_uint_custom::<u8>(2, VAR_UINT_7_LEN_BITS)
        .unwrap();
    builder.store_bytes(&[0, 1]).unwrap();
    builder
        .store_uint_custom::<u8>(0, VAR_UINT_7_LEN_BITS)
        .unwrap();

    let err = StorageUsed::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
}

#[test]
fn storage_used_rejects_varuint_7_length_seven() {
    let value = StorageUsed::new(BigUint::from(1u64) << 48, BigUint::from(0u8));

    let err = value.to_cell().unwrap_err();
    assert!(matches!(
        err,
        TlbError::NonCanonicalValue {
            schema: "StorageUsed.cells",
            ..
        }
    ));

    let mut builder = Builder::new();
    builder
        .store_uint_custom::<u8>(7, VAR_UINT_7_LEN_BITS)
        .unwrap();
    builder.store_bytes(&[1, 0, 0, 0, 0, 0, 0]).unwrap();
    builder
        .store_uint_custom::<u8>(0, VAR_UINT_7_LEN_BITS)
        .unwrap();

    let err = StorageUsed::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::NonCanonicalValue {
            schema: "StorageUsed.cells",
            ..
        }
    ));
}

#[test]
fn action_phase_roundtrips_without_optional_fields() {
    let value = sample_action_phase();

    assert_eq!(roundtrip(&value), value);
}

#[test]
fn action_phase_roundtrips_with_all_optional_fields_and_counters() {
    let value = TrActionPhase {
        success: false,
        valid: true,
        no_funds: true,
        status_change: AccStatusChange::Frozen,
        total_fwd_fees: Some(Grams::from(10_000)),
        total_action_fees: Some(Grams::from(20_000)),
        result_code: -14,
        result_arg: Some(32),
        tot_actions: 7,
        spec_actions: 1,
        skipped_actions: 2,
        msgs_created: 4,
        action_list_hash: [0xA5; 32],
        tot_msg_size: StorageUsed::new(BigUint::from(3u8), BigUint::from(777u16)),
    };

    assert_eq!(roundtrip(&value), value);
}

#[test]
fn action_phase_roundtrips_non_default_hash_and_message_size() {
    let mut action_list_hash = [0u8; 32];
    for (idx, byte) in action_list_hash.iter_mut().enumerate() {
        *byte = idx as u8;
    }
    let value = TrActionPhase {
        status_change: AccStatusChange::Deleted,
        result_code: 1,
        tot_actions: 255,
        spec_actions: 5,
        skipped_actions: 6,
        msgs_created: 250,
        action_list_hash,
        tot_msg_size: StorageUsed::new(BigUint::from(9u8), BigUint::from(1024u16)),
        ..sample_action_phase()
    };

    assert_eq!(roundtrip(&value), value);
}

#[test]
fn exact_action_phase_decode_rejects_trailing_data() {
    let value = sample_action_phase();
    let mut builder = Builder::new();
    value.store_tlb(&mut builder).unwrap();
    builder.store_bit(true).unwrap();
    let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));

    let mut builder = Builder::new();
    value.store_tlb(&mut builder).unwrap();
    builder.store_ref(Builder::new().build().unwrap()).unwrap();
    let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 0, refs: 1 }));
}

#[test]
fn action_phase_malformed_optional_grams_propagates_error() {
    let mut builder = Builder::new();
    builder.store_bit(true).unwrap();
    builder.store_bit(true).unwrap();
    builder.store_bit(false).unwrap();
    AccStatusChange::Unchanged.store_tlb(&mut builder).unwrap();
    builder.store_bit(true).unwrap();
    builder
        .store_uint_custom::<u8>(2, VAR_UINT_16_LEN_BITS)
        .unwrap();
    builder.store_bytes(&[0, 1]).unwrap();

    let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
}

#[test]
fn action_phase_malformed_storage_used_propagates_error() {
    let mut builder = Builder::new();
    store_action_phase_prefix_through_hash(&mut builder);
    builder
        .store_uint_custom::<u8>(2, VAR_UINT_7_LEN_BITS)
        .unwrap();
    builder.store_bytes(&[0, 1]).unwrap();
    builder
        .store_uint_custom::<u8>(0, VAR_UINT_7_LEN_BITS)
        .unwrap();

    let err = TrActionPhase::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::NonCanonicalValue { .. }));
}
