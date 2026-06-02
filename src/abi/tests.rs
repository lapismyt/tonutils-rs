use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;
    use crate::tlb::{MsgAddress, MsgAddressExt, TlbSerialize};
    use crate::tvm::{Address, Builder, TvmStackEntry};
    use num_bigint::{BigInt, BigUint};

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
                    key_bits: None,
                },
            ),
        ])))));

        assert_eq!(nested.validate(), Ok(()));

        let invalid = AbiType::Array(Box::new(AbiType::Map {
            key: Box::new(AbiType::Uint { bits: 0 }),
            value: Box::new(AbiType::Bool),
            key_bits: None,
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
    fn map_stack_conversion_roundtrips_in_canonical_key_order() {
        let map = AbiType::Map {
            key: Box::new(AbiType::Uint { bits: 32 }),
            value: Box::new(AbiType::String),
            key_bits: None,
        };
        let value = AbiValue::Map(vec![
            (
                AbiValue::Uint(BigUint::from(2u8)),
                AbiValue::String("b".to_string()),
            ),
            (
                AbiValue::Uint(BigUint::from(1u8)),
                AbiValue::String("a".to_string()),
            ),
        ]);
        let entry = value.to_stack_entry(&map).unwrap();
        assert!(matches!(entry, TvmStackEntry::Cell(_)));
        assert_eq!(
            abi_value_from_stack_entry(&map, &entry).unwrap(),
            AbiValue::Map(vec![
                (
                    AbiValue::Uint(BigUint::from(1u8)),
                    AbiValue::String("a".to_string())
                ),
                (
                    AbiValue::Uint(BigUint::from(2u8)),
                    AbiValue::String("b".to_string())
                ),
            ])
        );
    }

    #[test]
    fn map_conversion_rejects_duplicate_encoded_keys_and_unsupported_keys() {
        let map = AbiType::Map {
            key: Box::new(AbiType::Int { bits: 8 }),
            value: Box::new(AbiType::Bool),
            key_bits: None,
        };
        assert!(matches!(
            AbiValue::Map(vec![
                (AbiValue::Int(BigInt::from(-1)), AbiValue::Bool(true)),
                (AbiValue::Int(BigInt::from(-1)), AbiValue::Bool(false)),
            ])
            .to_stack_entry(&map)
            .unwrap_err(),
            AbiCodecError::DuplicateMapKey { .. }
        ));

        let map = AbiType::Map {
            key: Box::new(AbiType::Address),
            value: Box::new(AbiType::Bool),
            key_bits: None,
        };
        assert_eq!(
            AbiValue::Map(Vec::new()).to_stack_entry(&map).unwrap_err(),
            AbiCodecError::UnsupportedMapKey { ty: "address" }
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

    fn transfer_message_function() -> AbiFunction {
        AbiFunction {
            name: "transfer".to_string(),
            kind: AbiFunctionKind::InternalMessage,
            selector: AbiSelector::Opcode(0x0f8a_7ea5),
            inputs: vec![
                parameter("query_id", AbiType::Uint { bits: 64 }),
                parameter("amount", AbiType::Uint { bits: 257 }),
                parameter("recipient", AbiType::Address),
                parameter("notify", AbiType::Bool),
                parameter("comment", AbiType::Optional(Box::new(AbiType::String))),
            ],
            outputs: Vec::new(),
        }
    }

    #[test]
    fn message_body_roundtrips_opcode_and_supported_values() {
        let function = transfer_message_function();
        let values = vec![
            AbiValue::Uint(BigUint::from(7u8)),
            AbiValue::Uint(BigUint::from(1_000_000_000u64)),
            AbiValue::Address(Address::new(0, [0x33; 32])),
            AbiValue::Bool(true),
            AbiValue::Optional(Some(Box::new(AbiValue::String("hello".to_string())))),
        ];

        let body = encode_message_body(&function, &values).unwrap();
        assert_eq!(body.reference_count(), 1);
        assert_eq!(decode_message_body(&function, body).unwrap(), values);
    }

    #[test]
    fn message_body_roundtrips_none_selector_tuple_cell_and_empty_optional() {
        let mut payload = Builder::new();
        payload.store_u32(0xfeed_beef).unwrap();
        let payload = payload.build().unwrap();
        let function = AbiFunction {
            name: "external".to_string(),
            kind: AbiFunctionKind::ExternalMessage,
            selector: AbiSelector::None,
            inputs: vec![
                parameter(
                    "pair",
                    AbiType::Tuple(vec![
                        parameter("left", AbiType::Int { bits: 8 }),
                        parameter("right", AbiType::Cell),
                    ]),
                ),
                parameter(
                    "maybe",
                    AbiType::Optional(Box::new(AbiType::Uint { bits: 16 })),
                ),
            ],
            outputs: Vec::new(),
        };
        let values = vec![
            AbiValue::Tuple(vec![
                AbiValue::Int(BigInt::from(-5)),
                AbiValue::Cell(payload),
            ]),
            AbiValue::Optional(None),
        ];

        let body = encode_message_body(&function, &values).unwrap();
        assert_eq!(body.reference_count(), 1);
        assert_eq!(decode_message_body(&function, body).unwrap(), values);
    }

    #[test]
    fn payload_components_roundtrip_without_selector_prefix() {
        let parameters = vec![
            parameter("query_id", AbiType::Uint { bits: 64 }),
            parameter("recipient", AbiType::Address),
            parameter("memo", AbiType::Optional(Box::new(AbiType::String))),
        ];
        let values = vec![
            AbiValue::Uint(BigUint::from(7u8)),
            AbiValue::Address(Address::new(0, [0x33; 32])),
            AbiValue::Optional(Some(Box::new(AbiValue::String("hello".to_string())))),
        ];

        let payload = encode_payload_components(&parameters, &values).unwrap();

        assert_eq!(
            decode_payload_components(&parameters, payload).unwrap(),
            values
        );
    }

    #[test]
    fn event_payload_roundtrips_opcode_and_rejects_method_id() {
        let event = AbiEvent {
            name: "Transfer".to_string(),
            selector: AbiSelector::Opcode(0x0f8a_7ea5),
            fields: transfer_message_function().inputs,
        };
        let values = vec![
            AbiValue::Uint(BigUint::from(7u8)),
            AbiValue::Uint(BigUint::from(1_000_000_000u64)),
            AbiValue::Address(Address::new(0, [0x33; 32])),
            AbiValue::Bool(true),
            AbiValue::Optional(Some(Box::new(AbiValue::String("hello".to_string())))),
        ];

        let payload = encode_event_payload(&event, &values).unwrap();

        assert_eq!(payload.reference_count(), 1);
        assert_eq!(decode_event_payload(&event, payload).unwrap(), values);

        let bad = AbiEvent {
            name: "Bad".to_string(),
            selector: AbiSelector::MethodId(1),
            fields: Vec::new(),
        };
        assert_eq!(
            encode_event_payload(&bad, &[]).unwrap_err(),
            AbiCodecError::InvalidEventSelector {
                selector: AbiSelector::MethodId(1),
            }
        );
    }

    #[test]
    fn message_body_rejects_get_method_and_method_selector() {
        let get_method = valid_function();
        assert_eq!(
            encode_message_body(&get_method, &[]).unwrap_err(),
            AbiCodecError::InvalidMessageSelector {
                kind: AbiFunctionKind::GetMethod,
                selector: AbiSelector::MethodId(0x10001),
            }
        );

        let method_selector = AbiFunction {
            name: "bad".to_string(),
            kind: AbiFunctionKind::InternalMessage,
            selector: AbiSelector::MethodId(1),
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        assert_eq!(
            encode_message_body(&method_selector, &[]).unwrap_err(),
            AbiCodecError::InvalidMessageSelector {
                kind: AbiFunctionKind::InternalMessage,
                selector: AbiSelector::MethodId(1),
            }
        );
    }

    #[test]
    fn message_body_decode_rejects_opcode_mismatch_and_trailing_data() {
        let function = transfer_message_function();
        let mut wrong = Builder::new();
        wrong.store_u32(0xdead_beef).unwrap();
        let wrong = wrong.build().unwrap();
        assert_eq!(
            decode_message_body(&function, wrong).unwrap_err(),
            AbiCodecError::OpcodeMismatch {
                expected: 0x0f8a_7ea5,
                actual: 0xdead_beef,
            }
        );

        let no_inputs = AbiFunction {
            name: "ping".to_string(),
            kind: AbiFunctionKind::ExternalMessage,
            selector: AbiSelector::None,
            inputs: Vec::new(),
            outputs: Vec::new(),
        };
        let mut trailing = Builder::new();
        trailing.store_bit(true).unwrap();
        let trailing = trailing.build().unwrap();
        assert_eq!(
            decode_message_body(&no_inputs, trailing).unwrap_err(),
            AbiCodecError::TrailingBodyData { bits: 1, refs: 0 }
        );
    }

    #[test]
    fn message_body_rejects_arity_mismatch_and_unsupported_arrays() {
        let function = transfer_message_function();
        assert_eq!(
            encode_message_body(&function, &[]).unwrap_err(),
            AbiCodecError::ArityMismatch {
                kind: "message inputs",
                expected: 5,
                actual: 0,
            }
        );

        let array_function = AbiFunction {
            name: "array".to_string(),
            kind: AbiFunctionKind::InternalMessage,
            selector: AbiSelector::None,
            inputs: vec![parameter(
                "items",
                AbiType::Array(Box::new(AbiType::Uint { bits: 8 })),
            )],
            outputs: Vec::new(),
        };
        assert_eq!(
            encode_message_body(&array_function, &[AbiValue::Array(Vec::new())]).unwrap_err(),
            AbiCodecError::UnsupportedType { ty: "array" }
        );
    }

    #[test]
    fn get_method_helpers_encode_inputs_and_decode_outputs() {
        let function = valid_function();
        let address = Address::new(0, [0x44; 32]);
        let inputs =
            encode_get_method_inputs(&function, &[AbiValue::Address(address.clone())]).unwrap();
        assert_eq!(inputs.len(), 1);
        assert!(matches!(inputs[0], TvmStackEntry::Slice(_)));

        let outputs =
            decode_get_method_outputs(&function, &[TvmStackEntry::Int(BigInt::from(123u8))])
                .unwrap();
        assert_eq!(outputs, vec![AbiValue::Uint(BigUint::from(123u8))]);

        let message = transfer_message_function();
        assert_eq!(
            decode_get_method_outputs(&message, &[]).unwrap_err(),
            AbiCodecError::InvalidGetMethodSelector {
                kind: AbiFunctionKind::InternalMessage,
                selector: AbiSelector::Opcode(0x0f8a_7ea5),
            }
        );
    }

    #[test]
    fn get_method_helpers_reject_output_arity_mismatch() {
        let function = valid_function();
        assert_eq!(
            decode_get_method_outputs(&function, &[]).unwrap_err(),
            AbiCodecError::ArityMismatch {
                kind: "get-method outputs",
                expected: 1,
                actual: 0,
            }
        );
    }

    #[cfg(feature = "abi-json")]
    #[test]
    fn abi_json_parser_loads_definition_with_nested_types() {
        let definition = parse_abi_json_str(
            r#"{
                "name": "JettonWallet",
                "version": "0.1",
                "contracts": [{
                    "name": "JettonWallet",
                    "methods": [{
                        "name": "get_wallet_data",
                        "kind": "get_method",
                        "selector": { "method_id": "0x10001" },
                        "inputs": [{ "name": "owner", "type": "address" }],
                        "outputs": [
                            { "name": "balance", "type": "uint257" },
                            {
                                "name": "metadata",
                                "type": {
                                    "tuple": [
                                        { "name": "uri", "type": "optional<string>" },
                                        { "name": "raw", "type": { "array": "cell" } }
                                    ]
                                }
                            }
                        ]
                    }],
                    "events": [{
                        "name": "Transfer",
                        "selector": { "opcode": "0x0f8a7ea5" },
                        "fields": [{ "name": "amount", "type": "uint257" }]
                    }]
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(definition.name, "JettonWallet");
        let contract = &definition.contracts[0];
        assert_eq!(contract.methods[0].selector, AbiSelector::MethodId(0x10001));
        assert_eq!(
            contract.events[0].selector,
            AbiSelector::Opcode(0x0f8a_7ea5)
        );
        assert!(matches!(
            contract.methods[0].outputs[1].ty,
            AbiType::Tuple(_)
        ));
    }

    #[cfg(feature = "abi-json")]
    #[test]
    fn abi_json_parser_reports_precise_diagnostic_paths() {
        assert_eq!(
            parse_abi_json_str(
                r#"{
                    "name": "Bad",
                    "version": "0.1",
                    "contracts": [{
                        "name": "Bad",
                        "methods": [{
                            "name": "broken",
                            "kind": "get_method",
                            "outputs": [{ "name": "value", "type": "uint0" }]
                        }]
                    }]
                }"#,
            )
            .unwrap_err(),
            AbiJsonError::Model {
                source: AbiModelError::InvalidIntegerWidth {
                    kind: "integer",
                    bits: 0,
                    max: ABI_INTEGER_MAX_BITS,
                },
            }
        );

        assert_eq!(
            parse_abi_json_str(r#"{ "name": "Bad", "version": "0.1" }"#).unwrap_err(),
            AbiJsonError::MissingField {
                path: "$.contracts".to_string(),
            }
        );
    }

    #[cfg(feature = "abi-json")]
    #[test]
    fn abi_json_parser_rejects_ambiguous_selector() {
        assert_eq!(
            parse_abi_json_str(
                r#"{
                    "name": "Bad",
                    "version": "0.1",
                    "contracts": [{
                        "name": "Bad",
                        "methods": [{
                            "name": "broken",
                            "kind": "get_method",
                            "selector": { "method_id": 1, "opcode": 2 }
                        }]
                    }]
                }"#,
            )
            .unwrap_err(),
            AbiJsonError::AmbiguousSelector {
                path: "$.contracts[0].methods[0].selector".to_string(),
            }
        );
    }

    #[cfg(feature = "abi-json")]
    #[test]
    fn abi_json_parser_reports_unsupported_compatibility_shapes() {
        assert_eq!(
            parse_abi_json_str(
                r#"{
                    "ABI version": 2,
                    "name": "TactLike",
                    "version": "0.1",
                    "contracts": []
                }"#,
            )
            .unwrap_err(),
            AbiJsonError::UnsupportedShape {
                path: "$.ABI version".to_string(),
                shape: "ABI version".to_string(),
            }
        );

        assert_eq!(
            parse_abi_json_str(
                r#"{
                    "name": "Bad",
                    "version": "0.1",
                    "contracts": [{
                        "name": "Bad",
                        "methods": [{
                            "name": "broken",
                            "kind": "get_method",
                            "outputs": [{
                                "name": "value",
                                "type": { "kind": "simple", "type": "uint32" }
                            }]
                        }]
                    }]
                }"#,
            )
            .unwrap_err(),
            AbiJsonError::UnsupportedShape {
                path: "$.contracts[0].methods[0].outputs[0].type.kind".to_string(),
                shape: "kind".to_string(),
            }
        );
    }
}
