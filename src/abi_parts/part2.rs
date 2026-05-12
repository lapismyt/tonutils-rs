
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tlb::MsgAddressExt;

    fn parameter(name: &str, ty: AbiType) -> AbiParameter {
        AbiParameter {
            name: name.to_string(),
            ty,
            optional: false,
        }
    }

    fn valid_function() -> AbiFunction {
        AbiFunction {
            name: "get_wallet_data".to_string(),
            kind: AbiFunctionKind::GetMethod,
            selector: AbiSelector::MethodId(0x10001),
            inputs: vec![parameter("owner", AbiType::Address)],
            outputs: vec![parameter("balance", AbiType::Uint { bits: 257 })],
        }
    }

    fn valid_event() -> AbiEvent {
        AbiEvent {
            name: "Transfer".to_string(),
            selector: AbiSelector::Opcode(0x0f8a7ea5),
            fields: vec![
                parameter("query_id", AbiType::Uint { bits: 64 }),
                parameter("amount", AbiType::Uint { bits: 257 }),
            ],
        }
    }

    #[test]
    fn valid_function_event_contract_definition_pass_validation() {
        let definition = AbiDefinition {
            name: "JettonWallet".to_string(),
            version: "0.1".to_string(),
            contracts: vec![AbiContract {
                name: "JettonWallet".to_string(),
                methods: vec![valid_function()],
                events: vec![valid_event()],
            }],
        };

        assert_eq!(definition.validate(), Ok(()));
    }

    #[test]
    fn empty_required_names_fail_validation() {
        let mut function = valid_function();
        function.name = " ".to_string();
        assert_eq!(
            function.validate(),
            Err(AbiModelError::EmptyName {
                field: "function name"
            })
        );

        let mut event = valid_event();
        event.fields[0].name.clear();
        assert_eq!(
            event.validate(),
            Err(AbiModelError::EmptyName {
                field: "parameter name"
            })
        );

        let definition = AbiDefinition {
            name: String::new(),
            version: "0.1".to_string(),
            contracts: Vec::new(),
        };
        assert_eq!(
            definition.validate(),
            Err(AbiModelError::EmptyName {
                field: "ABI definition name"
            })
        );
    }

    #[test]
    fn integer_widths_reject_zero_and_greater_than_257() {
        assert_eq!(
            AbiType::Int { bits: 0 }.validate(),
            Err(AbiModelError::InvalidIntegerWidth {
                kind: "int",
                bits: 0,
                max: ABI_INTEGER_MAX_BITS,
            })
        );
        assert_eq!(
            AbiType::Uint { bits: 258 }.validate(),
            Err(AbiModelError::InvalidIntegerWidth {
                kind: "uint",
                bits: 258,
                max: ABI_INTEGER_MAX_BITS,
            })
        );
        assert_eq!(AbiType::Int { bits: 1 }.validate(), Ok(()));
        assert_eq!(AbiType::Uint { bits: 257 }.validate(), Ok(()));
    }

    #[test]
    fn nested_types_validate_recursively() {
        let nested = AbiType::Optional(Box::new(AbiType::Array(Box::new(AbiType::Tuple(vec![
            parameter(
                "balances",
                AbiType::Map {
                    key: Box::new(AbiType::Uint { bits: 32 }),
                    value: Box::new(AbiType::Optional(Box::new(AbiType::Cell))),
                },
            ),
        ])))));

        assert_eq!(nested.validate(), Ok(()));

        let invalid = AbiType::Array(Box::new(AbiType::Map {
            key: Box::new(AbiType::Uint { bits: 0 }),
            value: Box::new(AbiType::Bool),
        }));
        assert_eq!(
            invalid.validate(),
            Err(AbiModelError::InvalidIntegerWidth {
                kind: "uint",
                bits: 0,
                max: ABI_INTEGER_MAX_BITS,
            })
        );
    }

    #[test]
    fn selector_variants_preserve_exact_values() {
        let none = AbiSelector::None;
        let method = AbiSelector::MethodId(u64::MAX);
        let opcode = AbiSelector::Opcode(u32::MAX);

        assert_eq!(none, AbiSelector::None);
        assert_eq!(method, AbiSelector::MethodId(18_446_744_073_709_551_615));
        assert_eq!(opcode, AbiSelector::Opcode(4_294_967_295));
    }

    #[test]
    fn signed_and_unsigned_stack_values_respect_declared_widths() {
        let int1 = AbiType::Int { bits: 1 };
        assert_eq!(
            AbiValue::Int(BigInt::from(-1))
                .to_stack_entry(&int1)
                .unwrap(),
            TvmStackEntry::int(-1)
        );
        assert_eq!(
            AbiValue::Int(BigInt::from(0))
                .to_stack_entry(&int1)
                .unwrap(),
            TvmStackEntry::int(0)
        );
        assert_eq!(
            AbiValue::Int(BigInt::from(1))
                .to_stack_entry(&int1)
                .unwrap_err(),
            AbiCodecError::IntegerOutOfRange {
                kind: "int",
                bits: 1,
                value: "1".to_string(),
            }
        );

        let uint8 = AbiType::Uint { bits: 8 };
        assert_eq!(
            AbiValue::Uint(BigUint::from(255u16))
                .to_stack_entry(&uint8)
                .unwrap(),
            TvmStackEntry::int(255)
        );
        assert_eq!(
            AbiValue::Uint(BigUint::from(256u16))
                .to_stack_entry(&uint8)
                .unwrap_err(),
            AbiCodecError::IntegerOutOfRange {
                kind: "uint",
                bits: 8,
                value: "256".to_string(),
            }
        );

        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Int { bits: 0 }, &TvmStackEntry::int(0))
                .unwrap_err(),
            AbiCodecError::InvalidIntegerWidth {
                kind: "int",
                bits: 0,
                max: ABI_INTEGER_MAX_BITS,
            }
        );
    }

    #[test]
    fn bool_stack_values_use_tvm_canonical_minus_one_and_zero() {
        assert_eq!(
            AbiValue::Bool(true).to_stack_entry(&AbiType::Bool).unwrap(),
            TvmStackEntry::int(-1)
        );
        assert_eq!(
            AbiValue::Bool(false)
                .to_stack_entry(&AbiType::Bool)
                .unwrap(),
            TvmStackEntry::int(0)
        );
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Bool, &TvmStackEntry::int(-1)).unwrap(),
            AbiValue::Bool(true)
        );
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Bool, &TvmStackEntry::int(0)).unwrap(),
            AbiValue::Bool(false)
        );
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Bool, &TvmStackEntry::int(1)).unwrap_err(),
            AbiCodecError::InvalidBool {
                value: "1".to_string(),
            }
        );
    }

    #[test]
    fn address_encodes_to_slice_and_decodes_standard_msg_address() {
        let address = Address::new(0, [0x22; 32]);
        let value = AbiValue::Address(address.clone());
        let entry = value.to_stack_entry(&AbiType::Address).unwrap();

        assert!(matches!(entry, TvmStackEntry::Slice(_)));
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Address, &entry).unwrap(),
            value
        );
    }

    #[test]
    fn address_decode_rejects_non_canonical_address_payloads() {
        let cell = MsgAddress::Ext(MsgAddressExt::None).to_cell().unwrap();
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Address, &TvmStackEntry::Slice(cell)).unwrap_err(),
            AbiCodecError::MalformedAddress {
                reason: "external addresses are unsupported".to_string(),
            }
        );
    }

    #[test]
    fn bytes_and_string_roundtrip_through_snake_cells() {
        let bytes = (0..=255).cycle().take(180).collect::<Vec<_>>();
        let entry = AbiValue::Bytes(bytes.clone())
            .to_stack_entry(&AbiType::Bytes)
            .unwrap();
        assert!(matches!(entry, TvmStackEntry::Cell(_)));
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Bytes, &entry).unwrap(),
            AbiValue::Bytes(bytes)
        );

        let text = "hello TON ".repeat(80);
        let entry = AbiValue::String(text.clone())
            .to_stack_entry(&AbiType::String)
            .unwrap();
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::String, &entry).unwrap(),
            AbiValue::String(text)
        );
    }

    #[test]
    fn string_decode_rejects_invalid_utf8_snake_bytes() {
        let entry = AbiValue::Bytes(vec![0xff])
            .to_stack_entry(&AbiType::Bytes)
            .unwrap();
        assert!(matches!(
            abi_value_from_stack_entry(&AbiType::String, &entry),
            Err(AbiCodecError::InvalidUtf8 { .. })
        ));
    }

    #[test]
    fn snake_decode_rejects_non_byte_aligned_and_multi_ref_payloads() {
        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        let cell = builder.build().unwrap();
        assert!(matches!(
            abi_value_from_stack_entry(&AbiType::Bytes, &TvmStackEntry::Cell(cell)),
            Err(AbiCodecError::MalformedSnake { .. })
        ));

        let child = Builder::new().build().unwrap();
        let mut builder = Builder::new();
        builder.store_ref(child.clone()).unwrap();
        builder.store_ref(child).unwrap();
        let cell = builder.build().unwrap();
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Bytes, &TvmStackEntry::Cell(cell)).unwrap_err(),
            AbiCodecError::MalformedSnake {
                reason: "cell has 2 continuation references, expected at most 1".to_string(),
            }
        );
    }

    #[test]
    fn tuple_array_and_optional_values_recurse_through_stack_entries() {
        let ty = AbiType::Tuple(vec![
            parameter("flag", AbiType::Bool),
            parameter("items", AbiType::Array(Box::new(AbiType::Uint { bits: 8 }))),
            parameter("maybe", AbiType::Optional(Box::new(AbiType::String))),
        ]);
        let value = AbiValue::Tuple(vec![
            AbiValue::Bool(true),
            AbiValue::Array(vec![
                AbiValue::Uint(BigUint::from(1u8)),
                AbiValue::Uint(BigUint::from(2u8)),
            ]),
            AbiValue::Optional(Some(Box::new(AbiValue::String("ok".to_string())))),
        ]);

        let entry = value.to_stack_entry(&ty).unwrap();
        assert!(matches!(entry, TvmStackEntry::Tuple(_)));
        assert_eq!(abi_value_from_stack_entry(&ty, &entry).unwrap(), value);

        let none_ty = AbiType::Optional(Box::new(AbiType::Uint { bits: 8 }));
        let none = AbiValue::Optional(None);
        assert_eq!(none.to_stack_entry(&none_ty).unwrap(), TvmStackEntry::Null);
        assert_eq!(
            abi_value_from_stack_entry(&none_ty, &TvmStackEntry::Null).unwrap(),
            none
        );
    }

    #[test]
    fn mismatched_value_and_stack_types_return_deterministic_errors() {
        assert_eq!(
            AbiValue::Bool(true)
                .to_stack_entry(&AbiType::Uint { bits: 1 })
                .unwrap_err(),
            AbiCodecError::TypeMismatch {
                expected: "uint",
                actual: "bool",
            }
        );
        assert_eq!(
            abi_value_from_stack_entry(&AbiType::Cell, &TvmStackEntry::int(1)).unwrap_err(),
            AbiCodecError::TypeMismatch {
                expected: "cell",
                actual: "integer",
            }
        );

        let ty = AbiType::Tuple(vec![parameter("a", AbiType::Bool)]);
        assert_eq!(
            AbiValue::Tuple(vec![]).to_stack_entry(&ty).unwrap_err(),
            AbiCodecError::ArityMismatch {
                kind: "tuple",
                expected: 1,
                actual: 0,
            }
        );
    }

    #[test]
    fn map_and_unknown_stack_conversions_are_unsupported() {
        let map = AbiType::Map {
            key: Box::new(AbiType::Uint { bits: 32 }),
            value: Box::new(AbiType::Cell),
        };
        assert_eq!(
            AbiValue::Array(Vec::new())
                .to_stack_entry(&map)
                .unwrap_err(),
            AbiCodecError::UnsupportedType { ty: "map" }
        );
        assert_eq!(
            abi_value_from_stack_entry(
                &AbiType::Unknown("future".to_string()),
                &TvmStackEntry::Null
            )
            .unwrap_err(),
            AbiCodecError::UnsupportedType { ty: "unknown" }
        );
    }
}
