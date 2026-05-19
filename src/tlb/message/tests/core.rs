use super::*;
use crate::tlb::{TlbSerialize, expect_tag};
use crate::tvm::BitKey;

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

pub(super) fn std_address(byte: u8) -> Address {
    Address::new(0, [byte; 32])
}

pub(super) fn ext_in_info() -> CommonMsgInfo {
    CommonMsgInfo::ExternalIn {
        src: MsgAddressExt::None,
        dest: MsgAddressInt::std(std_address(0x11)),
        import_fee: Grams::from(0),
    }
}

pub(super) fn relaxed_internal_info(src: MsgAddress) -> CommonMsgInfoRelaxed {
    CommonMsgInfoRelaxed::Internal {
        ihr_disabled: true,
        bounce: false,
        bounced: false,
        src,
        dest: MsgAddressInt::std(std_address(0x22)),
        value: CurrencyCollection::grams(Grams::from(7)),
        extra_flags: BigUint::from(2u8),
        fwd_fee: Grams::from(3),
        created_lt: 4,
        created_at: 5,
    }
}

#[test]
pub(super) fn std_internal_address_roundtrips() {
    let value = MsgAddressInt::Std {
        anycast: None,
        address: Address::new(-1, [0xAA; 32]),
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
pub(super) fn variable_internal_address_roundtrips() {
    let value = MsgAddressInt::Var {
        anycast: Some(Anycast {
            depth: 3,
            rewrite_pfx: vec![0b1010_0000],
        }),
        workchain_id: -239,
        address: vec![0b1101_0000],
        bit_len: 4,
    };
    assert_eq!(roundtrip(&value), value);
}

#[test]
pub(super) fn external_addresses_roundtrip() {
    assert_eq!(roundtrip(&MsgAddressExt::None), MsgAddressExt::None);
    let raw = MsgAddressExt::Extern {
        data: vec![0b1010_0000],
        bit_len: 4,
    };
    assert_eq!(roundtrip(&raw), raw);
}

#[test]
pub(super) fn relaxed_msg_address_roundtrips_internal_and_external_forms() {
    let std = MsgAddress::Int(MsgAddressInt::std(std_address(0x10)));
    assert_eq!(roundtrip(&std), std);

    let var = MsgAddress::Int(MsgAddressInt::Var {
        anycast: None,
        workchain_id: -1,
        address: vec![0b1110_0000],
        bit_len: 3,
    });
    assert_eq!(roundtrip(&var), var);

    assert_eq!(
        roundtrip(&MsgAddress::Ext(MsgAddressExt::None)),
        MsgAddress::Ext(MsgAddressExt::None)
    );

    let raw = MsgAddress::Ext(MsgAddressExt::Extern {
        data: vec![0b0110_0000],
        bit_len: 3,
    });
    assert_eq!(roundtrip(&raw), raw);
}

#[test]
pub(super) fn malformed_anycast_depth_is_rejected() {
    for depth in [0u64, 31] {
        let mut builder = Builder::new();
        builder.store_uint_custom::<u8>(depth as u8, 5).unwrap();
        if depth > 0 {
            builder
                .store_bits(&vec![0; (depth as usize).div_ceil(8)], depth as usize)
                .unwrap();
        }
        let err = Anycast::from_cell(builder.build().unwrap()).unwrap_err();
        assert!(matches!(
            err,
            TlbError::CustomSchema {
                schema: "Anycast",
                ..
            }
        ));
    }
}

#[test]
pub(super) fn grams_canonical_encodings_roundtrip() {
    assert_eq!(roundtrip(&Grams::from(0)), Grams::from(0));
    assert_eq!(
        roundtrip(&Grams::from(1_000_000_000)),
        Grams::from(1_000_000_000)
    );
}

#[test]
pub(super) fn currency_collection_roundtrips_empty_and_extra_currency() {
    let empty = CurrencyCollection::grams(Grams::from(123));
    assert_eq!(roundtrip(&empty), empty);

    let mut other = HashmapE::new(32);
    other
        .insert_bit_key(BitKey::from_u64(7, 32).unwrap(), BigUint::from(42u8))
        .unwrap();
    let value = CurrencyCollection {
        grams: Grams::from(1),
        other,
    };
    let decoded = roundtrip(&value);
    assert_eq!(
        decoded
            .other
            .get_bit_key(&BitKey::from_u64(7, 32).unwrap())
            .unwrap(),
        Some(&BigUint::from(42u8))
    );
    assert_eq!(decoded, value);
}

#[test]
pub(super) fn state_init_empty_roundtrips() {
    assert_eq!(roundtrip(&StateInit::empty()), StateInit::empty());
}

#[test]
pub(super) fn state_init_references_preserve_hashes() {
    let code = cell_with_bits(&[0xAA], 8);
    let data = cell_with_bits(&[0xBC], 6);
    let library = cell_with_bits(&[0xF0], 4);
    let value = StateInit {
        fixed_prefix_length: Some(5),
        special: Some(TickTock {
            tick: true,
            tock: false,
        }),
        code: Some(code.clone()),
        data: Some(data.clone()),
        library: Some(library.clone()),
    };
    let decoded = roundtrip(&value);
    assert_eq!(decoded.code.unwrap().hash(), code.hash());
    assert_eq!(decoded.data.unwrap().hash(), data.hash());
    assert_eq!(decoded.library.unwrap().hash(), library.hash());
}

#[test]
pub(super) fn common_msg_info_variants_roundtrip() {
    let internal = CommonMsgInfo::Internal {
        ihr_disabled: true,
        bounce: true,
        bounced: false,
        src: MsgAddressInt::std(std_address(0x01)),
        dest: MsgAddressInt::std(std_address(0x02)),
        value: CurrencyCollection::grams(Grams::from(100)),
        extra_flags: BigUint::from(3u8),
        fwd_fee: Grams::from(9),
        created_lt: 10,
        created_at: 11,
    };
    assert_eq!(roundtrip(&internal), internal);

    let ext_in = ext_in_info();
    assert_eq!(roundtrip(&ext_in), ext_in);

    let ext_out = CommonMsgInfo::ExternalOut {
        src: MsgAddressInt::std(std_address(0x33)),
        dest: MsgAddressExt::Extern {
            data: vec![0b1000_0000],
            bit_len: 1,
        },
        created_lt: 44,
        created_at: 55,
    };
    assert_eq!(roundtrip(&ext_out), ext_out);
}

#[test]
pub(super) fn tag_mismatch_failures_are_reported() {
    let mut builder = Builder::new();
    store_tag(&mut builder, "00").unwrap();
    let err = MsgAddressInt::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "MsgAddressInt",
            ..
        }
    ));

    let mut builder = Builder::new();
    store_tag(&mut builder, "11").unwrap();
    let err = MsgAddressExt::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "MsgAddressExt",
            ..
        }
    ));

    let mut builder = Builder::new();
    store_tag(&mut builder, "10").unwrap();
    let mut slice = Slice::new(builder.build().unwrap());
    expect_tag(&mut slice, "manual$11", "11").unwrap_err();
}

#[test]
pub(super) fn msg_address_truncated_tag_is_rejected() {
    let mut builder = Builder::new();
    builder.store_bit(true).unwrap();
    let err = MsgAddress::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "MsgAddress",
            actual_bits,
            ..
        } if actual_bits == "1"
    ));
}

#[test]
pub(super) fn simple_lib_and_state_init_with_libs_roundtrip() {
    assert_eq!(
        roundtrip(&StateInitWithLibs::empty()),
        StateInitWithLibs::empty()
    );

    let root = cell_with_bits(&[0xCE], 8);
    let lib = SimpleLib {
        public: true,
        root: root.clone(),
    };
    assert_eq!(roundtrip(&lib).root.hash(), root.hash());

    let mut library = HashmapE::new(256);
    library
        .insert_bit_key(BitKey::from_bits(vec![0xAB; 32], 256).unwrap(), lib)
        .unwrap();
    let value = StateInitWithLibs {
        fixed_prefix_length: Some(3),
        special: Some(TickTock {
            tick: false,
            tock: true,
        }),
        code: Some(cell_with_bits(&[0x11], 8)),
        data: Some(cell_with_bits(&[0x22], 8)),
        library,
    };
    let decoded = roundtrip(&value);
    let key = BitKey::from_bits(vec![0xAB; 32], 256).unwrap();
    let decoded_lib = decoded
        .library
        .get_bit_key(&key)
        .unwrap()
        .expect("library entry");
    assert_eq!(decoded_lib.root.hash(), root.hash());
    assert_eq!(decoded, value);
}

#[test]
pub(super) fn common_msg_info_relaxed_variants_roundtrip_and_reject_external_in() {
    let internal_src =
        relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x44))));
    assert_eq!(roundtrip(&internal_src), internal_src);

    let external_src = relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::Extern {
        data: vec![0b1010_0000],
        bit_len: 4,
    }));
    assert_eq!(roundtrip(&external_src), external_src);

    let ext_out = CommonMsgInfoRelaxed::ExternalOut {
        src: MsgAddress::Int(MsgAddressInt::std(std_address(0x55))),
        dest: MsgAddressExt::None,
        created_lt: 6,
        created_at: 7,
    };
    assert_eq!(roundtrip(&ext_out), ext_out);

    let mut builder = Builder::new();
    store_tag(&mut builder, "10").unwrap();
    let err = CommonMsgInfoRelaxed::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(
        err,
        TlbError::TagMismatch {
            constructor: "CommonMsgInfoRelaxed",
            actual_bits,
            ..
        } if actual_bits == "10"
    ));
}

#[test]
pub(super) fn external_in_message_with_inline_empty_body_roundtrips() {
    let body = Builder::new().build().unwrap();
    let message = Message {
        info: ext_in_info(),
        init: None,
        body: Either::Left(body.clone()),
    };
    let decoded = roundtrip(&message);
    assert_eq!(decoded, message);
    assert_eq!(
        match decoded.body {
            Either::Left(cell) => cell.hash(),
            Either::Right(_) => [0; 32],
        },
        body.hash()
    );
}

#[test]
pub(super) fn message_with_referenced_state_init_roundtrips() {
    let init = StateInit {
        code: Some(cell_with_bits(&[0xAB], 8)),
        ..StateInit::empty()
    };
    let message = Message {
        info: ext_in_info(),
        init: Some(Either::Right(init.clone())),
        body: Either::Left(Builder::new().build().unwrap()),
    };
    assert_eq!(roundtrip(&message), message);
}

#[test]
pub(super) fn message_with_referenced_body_roundtrips() {
    let body = cell_with_bits(&[0xAB, 0xC0], 10);
    let message = Message {
        info: ext_in_info(),
        init: None,
        body: Either::Right(body.clone()),
    };
    let decoded = roundtrip(&message);
    assert_eq!(decoded, message);
    assert_eq!(
        match decoded.body {
            Either::Right(cell) => cell.hash(),
            Either::Left(_) => [0; 32],
        },
        body.hash()
    );
}

#[test]
pub(super) fn exact_message_decode_rejects_trailing_data_after_referenced_body() {
    let body = cell_with_bits(&[0xAB], 8);
    let mut builder = Builder::new();
    Message {
        info: ext_in_info(),
        init: None,
        body: Either::Right(body),
    }
    .store_tlb(&mut builder)
    .unwrap();
    builder.store_bit(true).unwrap();
    let err = Message::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
}

#[test]
pub(super) fn relaxed_message_with_inline_empty_body_roundtrips() {
    let body = Builder::new().build().unwrap();
    let message = MessageRelaxed {
        info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
        init: None,
        body: Either::Left(body.clone()),
    };
    let decoded = roundtrip(&message);
    assert_eq!(decoded, message);
    assert_eq!(
        match decoded.body {
            Either::Left(cell) => cell.hash(),
            Either::Right(_) => [0; 32],
        },
        body.hash()
    );
}

#[test]
pub(super) fn relaxed_message_with_referenced_state_init_roundtrips() {
    let init = StateInit {
        data: Some(cell_with_bits(&[0xCD], 8)),
        ..StateInit::empty()
    };
    let message = MessageRelaxed {
        info: relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x66)))),
        init: Some(Either::Right(init.clone())),
        body: Either::Left(Builder::new().build().unwrap()),
    };
    assert_eq!(roundtrip(&message), message);
}

#[test]
pub(super) fn relaxed_message_with_referenced_body_roundtrips() {
    let body = cell_with_bits(&[0xAD, 0x80], 9);
    let message = MessageRelaxed {
        info: relaxed_internal_info(MsgAddress::Int(MsgAddressInt::std(std_address(0x77)))),
        init: None,
        body: Either::Right(body.clone()),
    };
    let decoded = roundtrip(&message);
    assert_eq!(decoded, message);
    assert_eq!(
        match decoded.body {
            Either::Right(cell) => cell.hash(),
            Either::Left(_) => [0; 32],
        },
        body.hash()
    );
}

#[test]
pub(super) fn exact_relaxed_message_decode_rejects_trailing_data_after_referenced_body() {
    let body = cell_with_bits(&[0xEF], 8);
    let mut builder = Builder::new();
    MessageRelaxed {
        info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
        init: None,
        body: Either::Right(body),
    }
    .store_tlb(&mut builder)
    .unwrap();
    builder.store_bit(false).unwrap();
    let err = MessageRelaxed::from_cell(builder.build().unwrap()).unwrap_err();
    assert!(matches!(err, TlbError::TrailingData { bits: 1, refs: 0 }));
}

#[test]
pub(super) fn out_action_send_msg_roundtrips_referenced_relaxed_message() {
    let body = cell_with_bits(&[0x42], 8);
    let out_msg = MessageRelaxed {
        info: relaxed_internal_info(MsgAddress::Ext(MsgAddressExt::None)),
        init: None,
        body: Either::Right(body.clone()),
    };
    let action = OutAction::SendMsg {
        mode: 3,
        out_msg: out_msg.clone(),
    };

    let decoded = roundtrip(&action);
    assert_eq!(decoded, action);
    match decoded {
        OutAction::SendMsg { out_msg, .. } => match out_msg.body {
            Either::Right(decoded_body) => assert_eq!(decoded_body.hash(), body.hash()),
            Either::Left(_) => panic!("expected referenced body"),
        },
        _ => panic!("expected send message action"),
    }
}

#[test]
pub(super) fn out_action_set_code_preserves_cell_hash() {
    let code = cell_with_bits(&[0xAD, 0x80], 9);
    let action = OutAction::SetCode {
        new_code: code.clone(),
    };

    let decoded = roundtrip(&action);
    match decoded {
        OutAction::SetCode { new_code } => assert_eq!(new_code.hash(), code.hash()),
        _ => panic!("expected set code action"),
    }
}
