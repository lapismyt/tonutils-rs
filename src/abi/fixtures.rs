use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tvm::{Address, Cell, TvmStack, TvmStackEntry, hex_to_boc};
    use num_bigint::{BigInt, BigUint};
    use serde::Deserialize;
    use serde_json::Value;
    use std::str::FromStr;
    use std::sync::Arc;

    const FIXTURES_JSON: &str = include_str!("../../fixtures/abi/contracts.json");

    #[derive(Debug, Deserialize)]
    struct FixtureSet {
        schema_revision: u32,
        source: String,
        capture_date: String,
        fixtures: Vec<Fixture>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct Fixture {
        name: String,
        kind: String,
        evidence_kind: String,
        source: String,
        source_url: Option<String>,
        source_commit: Option<String>,
        network: Option<String>,
        account: Option<String>,
        block_id: Option<Value>,
        method_id: Option<String>,
        capture_command: Option<String>,
        compat_reference: Option<String>,
        abi_json: Value,
        function: String,
        input_values: Vec<Value>,
        expected_decoded_outputs: Option<Vec<Value>>,
        input_stack_boc_hex: Option<String>,
        input_stack_root_hash: Option<String>,
        output_stack_boc_hex: Option<String>,
        output_stack_root_hash: Option<String>,
        message_body_boc_hex: Option<String>,
        root_hash: Option<String>,
        expected_decoded_inputs: Option<Vec<Value>>,
    }

    fn fixtures() -> FixtureSet {
        serde_json::from_str(FIXTURES_JSON).unwrap()
    }

    #[test]
    fn abi_fixture_json_schema_sanity() {
        let set = fixtures();
        assert_eq!(set.schema_revision, 1);
        assert_eq!(set.capture_date, "2026-05-14");
        assert!(!set.source.trim().is_empty());
        assert!(set.fixtures.len() >= 4);

        for fixture in &set.fixtures {
            assert!(!fixture.name.trim().is_empty());
            assert!(!fixture.kind.trim().is_empty());
            assert!(matches!(
                fixture.evidence_kind.as_str(),
                "synthetic" | "captured_or_opt_in"
            ));
            assert!(!fixture.source.trim().is_empty());
            assert!(
                fixture
                    .source_url
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            );
            assert!(
                fixture
                    .source_commit
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            );
            assert!(
                fixture
                    .network
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            );
            assert!(!fixture.function.trim().is_empty());
            if function_has_inputs(fixture) {
                assert!(!fixture.input_values.is_empty());
            }
            assert!(parse_definition(fixture).validate().is_ok());
            for hex in [
                fixture.input_stack_boc_hex.as_deref(),
                fixture.input_stack_root_hash.as_deref(),
                fixture.output_stack_boc_hex.as_deref(),
                fixture.output_stack_root_hash.as_deref(),
                fixture.message_body_boc_hex.as_deref(),
                fixture.root_hash.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert!(hex::decode(hex).is_ok(), "{}", fixture.name);
            }
        }

        assert!(
            set.fixtures
                .iter()
                .any(|fixture| fixture.evidence_kind == "synthetic")
        );
        assert!(
            set.fixtures
                .iter()
                .any(|fixture| fixture.evidence_kind == "captured_or_opt_in")
        );
    }

    #[test]
    fn get_method_fixture_encodes_inputs_and_decodes_outputs() {
        let fixture = named_fixture("get_method_address_uint");
        let function = fixture_function(&fixture);
        let values = parse_values(&fixture.input_values, &function.inputs);
        let entries = encode_get_method_inputs(&function, &values).unwrap();
        let stack = TvmStack::new(entries);

        assert_eq!(
            hex::encode(stack.to_boc().unwrap()),
            fixture.input_stack_boc_hex.as_deref().unwrap()
        );
        assert_eq!(
            hex::encode(stack.to_cell().unwrap().hash()),
            fixture.input_stack_root_hash.as_deref().unwrap()
        );

        let output_bytes = hex::decode(fixture.output_stack_boc_hex.as_deref().unwrap()).unwrap();
        let output_stack = TvmStack::from_boc(&output_bytes).unwrap();
        assert_eq!(
            hex::encode(output_stack.to_cell().unwrap().hash()),
            fixture.output_stack_root_hash.as_deref().unwrap()
        );
        let decoded = decode_get_method_outputs(&function, output_stack.entries()).unwrap();
        let expected = parse_values(
            fixture.expected_decoded_outputs.as_deref().unwrap(),
            &function.outputs,
        );
        assert_eq!(decoded, expected);
    }

    #[test]
    fn message_body_fixtures_encode_decode_and_reencode_exactly() {
        for fixture_name in [
            "external_opcode_scalar_refs",
            "internal_no_selector_tuple_optional",
        ] {
            let fixture = named_fixture(fixture_name);
            let function = fixture_function(&fixture);
            let values = parse_values(&fixture.input_values, &function.inputs);
            let body = encode_message_body(&function, &values).unwrap();

            assert_eq!(
                crate::tvm::boc_to_hex(&body, false).unwrap(),
                fixture.message_body_boc_hex.as_deref().unwrap()
            );
            assert_eq!(
                hex::encode(body.hash()),
                fixture.root_hash.as_deref().unwrap()
            );

            let fixture_body =
                hex_to_boc(fixture.message_body_boc_hex.as_deref().unwrap()).unwrap();
            assert_eq!(
                hex::encode(fixture_body.hash()),
                fixture.root_hash.as_deref().unwrap()
            );
            let decoded = decode_message_body(&function, fixture_body).unwrap();
            let expected = parse_values(
                fixture.expected_decoded_inputs.as_deref().unwrap(),
                &function.inputs,
            );
            assert_eq!(decoded, expected);

            let reencoded = encode_message_body(&function, &decoded).unwrap();
            assert_eq!(
                crate::tvm::boc_to_hex(&reencoded, false).unwrap(),
                fixture.message_body_boc_hex.as_deref().unwrap()
            );
        }
    }

    #[test]
    fn event_payload_fixture_encodes_decodes_and_reencodes_exactly() {
        let fixture = named_fixture("event_opcode_scalar_address_optional_map");
        let event = fixture_event(&fixture);
        let values = parse_values(&fixture.input_values, &event.fields);
        let payload = encode_event_payload(&event, &values).unwrap();

        assert_eq!(
            crate::tvm::boc_to_hex(&payload, false).unwrap(),
            fixture.message_body_boc_hex.as_deref().unwrap()
        );
        assert_eq!(
            hex::encode(payload.hash()),
            fixture.root_hash.as_deref().unwrap()
        );

        let fixture_payload = hex_to_boc(fixture.message_body_boc_hex.as_deref().unwrap()).unwrap();
        assert_eq!(
            hex::encode(fixture_payload.hash()),
            fixture.root_hash.as_deref().unwrap()
        );
        let decoded = decode_event_payload(&event, fixture_payload).unwrap();
        let expected = parse_values(
            fixture.expected_decoded_inputs.as_deref().unwrap(),
            &event.fields,
        );
        assert_eq!(decoded, expected);

        let reencoded = encode_event_payload(&event, &decoded).unwrap();
        assert_eq!(
            crate::tvm::boc_to_hex(&reencoded, false).unwrap(),
            fixture.message_body_boc_hex.as_deref().unwrap()
        );
    }

    #[test]
    fn map_dictionary_fixtures_roundtrip_stack_and_message_body() {
        for fixture_name in ["map_get_method", "map_message_body"] {
            let fixture = named_fixture(fixture_name);
            let function = fixture_function(&fixture);
            let values = parse_values(&fixture.input_values, &function.inputs);

            match function.kind {
                AbiFunctionKind::GetMethod => {
                    let entries = encode_get_method_inputs(&function, &values).unwrap();
                    assert_eq!(entries.len(), 1);
                    assert_eq!(
                        AbiValue::from_stack_entry(&function.inputs[0].ty, &entries[0]).unwrap(),
                        values[0]
                    );
                }
                AbiFunctionKind::InternalMessage | AbiFunctionKind::ExternalMessage => {
                    let body = encode_message_body(&function, &values).unwrap();
                    assert_eq!(decode_message_body(&function, body).unwrap(), values);
                }
            }
        }
    }

    #[test]
    fn opt_in_get_method_templates_accept_declared_abi_workflows() {
        let seqno = named_fixture("wallet_seqno_no_input_opt_in");
        let function = fixture_function(&seqno);
        let values = parse_values(&seqno.input_values, &function.inputs);
        assert!(
            encode_get_method_inputs(&function, &values)
                .unwrap()
                .is_empty()
        );

        let decoded =
            decode_get_method_outputs(&function, &[TvmStackEntry::Int(BigInt::from(42u8))])
                .unwrap();
        assert_eq!(decoded, vec![AbiValue::Uint(BigUint::from(42u8))]);

        let wallet_address = named_fixture("jetton_get_wallet_address_opt_in");
        let function = fixture_function(&wallet_address);
        let values = parse_values(&wallet_address.input_values, &function.inputs);
        let entries = encode_get_method_inputs(&function, &values).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            AbiValue::from_stack_entry(&function.inputs[0].ty, &entries[0]).unwrap(),
            values[0]
        );

        let wallet = AbiValue::Address(Address::new(0, [0x55; 32]));
        let output = wallet.to_stack_entry(&function.outputs[0].ty).unwrap();
        let decoded = decode_get_method_outputs(&function, &[output]).unwrap();
        assert_eq!(decoded, vec![wallet]);
    }

    fn named_fixture(name: &str) -> Fixture {
        fixtures()
            .fixtures
            .into_iter()
            .find(|fixture| fixture.name == name)
            .unwrap_or_else(|| panic!("missing fixture {name}"))
    }

    fn fixture_function(fixture: &Fixture) -> AbiFunction {
        let definition = parse_definition(fixture);
        definition
            .contracts
            .iter()
            .flat_map(|contract| contract.methods.iter())
            .find(|function| function.name == fixture.function)
            .unwrap_or_else(|| panic!("missing function {}", fixture.function))
            .clone()
    }

    fn fixture_event(fixture: &Fixture) -> AbiEvent {
        let definition = parse_definition(fixture);
        definition
            .contracts
            .iter()
            .flat_map(|contract| contract.events.iter())
            .find(|event| event.name == fixture.function)
            .unwrap_or_else(|| panic!("missing event {}", fixture.function))
            .clone()
    }

    fn function_has_inputs(fixture: &Fixture) -> bool {
        if fixture.kind == "event_payload" {
            !fixture_event(fixture).fields.is_empty()
        } else {
            !fixture_function(fixture).inputs.is_empty()
        }
    }

    fn parse_definition(fixture: &Fixture) -> AbiDefinition {
        parse_abi_json_str(&fixture.abi_json.to_string()).unwrap()
    }

    fn parse_values(values: &[Value], parameters: &[AbiParameter]) -> Vec<AbiValue> {
        assert_eq!(values.len(), parameters.len());
        values
            .iter()
            .zip(parameters)
            .map(|(value, parameter)| parse_value(value, &parameter.ty))
            .collect()
    }

    fn parse_value(value: &Value, ty: &AbiType) -> AbiValue {
        match ty {
            AbiType::Int { .. } => AbiValue::Int(parse_bigint(value)),
            AbiType::Uint { .. } => AbiValue::Uint(parse_biguint(value)),
            AbiType::Bool => AbiValue::Bool(value.as_bool().unwrap()),
            AbiType::Bytes => AbiValue::Bytes(hex::decode(required_str(value)).unwrap()),
            AbiType::String => AbiValue::String(required_str(value).to_owned()),
            AbiType::Address => {
                let object = value.as_object().unwrap();
                let workchain = object.get("workchain").unwrap().as_i64().unwrap() as i8;
                let hash = hex::decode(object.get("hash").unwrap().as_str().unwrap()).unwrap();
                let hash: [u8; 32] = hash.try_into().unwrap();
                AbiValue::Address(Address::new(workchain, hash))
            }
            AbiType::Cell => AbiValue::Cell(parse_cell(value)),
            AbiType::Slice => AbiValue::Slice(parse_cell(value)),
            AbiType::Tuple(fields) => {
                let items = value.as_array().unwrap();
                let values = items
                    .iter()
                    .zip(fields)
                    .map(|(value, field)| parse_value(value, &field.ty))
                    .collect();
                AbiValue::Tuple(values)
            }
            AbiType::Array(item_ty) => AbiValue::Array(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| parse_value(value, item_ty))
                    .collect(),
            ),
            AbiType::Optional(item_ty) if value.is_null() => AbiValue::Optional(None),
            AbiType::Optional(item_ty) => {
                AbiValue::Optional(Some(Box::new(parse_value(value, item_ty))))
            }
            AbiType::Map {
                key,
                value: value_ty,
                ..
            } => AbiValue::Map(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|entry| {
                        let object = entry.as_object().unwrap();
                        (
                            parse_value(object.get("key").unwrap(), key),
                            parse_value(object.get("value").unwrap(), value_ty),
                        )
                    })
                    .collect(),
            ),
            AbiType::Unknown(name) => panic!("unsupported unknown fixture type {name}"),
        }
    }

    fn parse_bigint(value: &Value) -> BigInt {
        match value {
            Value::Number(number) => BigInt::from_str(&number.to_string()).unwrap(),
            Value::String(value) => BigInt::from_str(value).unwrap(),
            _ => panic!("expected integer fixture value"),
        }
    }

    fn parse_biguint(value: &Value) -> BigUint {
        match value {
            Value::Number(number) => BigUint::from_str(&number.to_string()).unwrap(),
            Value::String(value) => BigUint::from_str(value).unwrap(),
            _ => panic!("expected unsigned integer fixture value"),
        }
    }

    fn parse_cell(value: &Value) -> Arc<Cell> {
        hex_to_boc(required_str(value)).unwrap()
    }

    fn required_str(value: &Value) -> &str {
        value.as_str().unwrap()
    }
}
