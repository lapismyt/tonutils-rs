    use super::*;
    use crate::tlb::{TlbSerialize, expect_tag};
    use crate::tvm::BitKey;

    fn roundtrip<T>(value: &T) -> T
    where
        T: TlbSerialize + TlbDeserialize + PartialEq + std::fmt::Debug,
    {
        T::from_cell(value.to_cell().unwrap()).unwrap()
    }

    fn cell_with_bits(data: &[u8], bit_len: usize) -> Arc<Cell> {
        let mut builder = Builder::new();
        builder.store_bits(data, bit_len).unwrap();
        builder.build().unwrap()
    }

    fn std_address(byte: u8) -> Address {
        Address::new(0, [byte; 32])
    }

    fn ext_in_info() -> CommonMsgInfo {
        CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(std_address(0x11)),
            import_fee: Grams::from(0),
        }
    }

    fn relaxed_internal_info(src: MsgAddress) -> CommonMsgInfoRelaxed {
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
    fn std_internal_address_roundtrips() {
        let value = MsgAddressInt::Std {
            anycast: None,
            address: Address::new(-1, [0xAA; 32]),
        };
        assert_eq!(roundtrip(&value), value);
    }

    #[test]
    fn variable_internal_address_roundtrips() {
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
    fn external_addresses_roundtrip() {
        assert_eq!(roundtrip(&MsgAddressExt::None), MsgAddressExt::None);
        let raw = MsgAddressExt::Extern {
            data: vec![0b1010_0000],
            bit_len: 4,
        };
        assert_eq!(roundtrip(&raw), raw);
    }

    #[test]
    fn relaxed_msg_address_roundtrips_internal_and_external_forms() {
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
    fn malformed_anycast_depth_is_rejected() {
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
    fn grams_canonical_encodings_roundtrip() {
        assert_eq!(roundtrip(&Grams::from(0)), Grams::from(0));
        assert_eq!(
            roundtrip(&Grams::from(1_000_000_000)),
            Grams::from(1_000_000_000)
        );
    }

    #[test]
    fn currency_collection_roundtrips_empty_and_extra_currency() {
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
    fn state_init_empty_roundtrips() {
        assert_eq!(roundtrip(&StateInit::empty()), StateInit::empty());
    }

    #[test]
    fn state_init_references_preserve_hashes() {
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
    fn common_msg_info_variants_roundtrip() {
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
    fn tag_mismatch_failures_are_reported() {
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
    fn msg_address_truncated_tag_is_rejected() {
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
    fn simple_lib_and_state_init_with_libs_roundtrip() {
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
    fn common_msg_info_relaxed_variants_roundtrip_and_reject_external_in() {
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
    fn external_in_message_with_inline_empty_body_roundtrips() {
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
    fn message_with_referenced_state_init_roundtrips() {
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
    fn message_with_referenced_body_roundtrips() {
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
    fn exact_message_decode_rejects_trailing_data_after_referenced_body() {
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
    fn relaxed_message_with_inline_empty_body_roundtrips() {
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
    fn relaxed_message_with_referenced_state_init_roundtrips() {
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
    fn relaxed_message_with_referenced_body_roundtrips() {
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
    fn exact_relaxed_message_decode_rejects_trailing_data_after_referenced_body() {
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
    fn out_action_send_msg_roundtrips_referenced_relaxed_message() {
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
    fn out_action_set_code_preserves_cell_hash() {
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

