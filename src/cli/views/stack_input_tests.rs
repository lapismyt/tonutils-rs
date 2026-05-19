use super::*;

fn empty_boc_hex() -> String {
    let cell = crate::tvm::CellBuilder::new().build().unwrap();
    hex::encode(crate::tvm::serialize_boc(&cell, false).unwrap())
}

const RAW_ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn parses_high_level_call_stack_json_cli_arg() {
    let cli = Cli::try_parse_from([
        "tonutils",
        "call",
        RAW_ADDRESS,
        "seqno",
        "--stack-json",
        r#"[{"type":"int","value":"1"}]"#,
    ])
    .unwrap();

    match cli.command {
        Commands::Call(args) => {
            assert!(args.args.is_empty());
            assert_eq!(
                args.stack_json.as_deref(),
                Some(r#"[{"type":"int","value":"1"}]"#)
            );
        }
        _ => panic!("unexpected command"),
    }
}

#[test]
fn rejects_mixed_high_level_call_stack_inputs() {
    assert!(
        Cli::try_parse_from([
            "tonutils",
            "call",
            RAW_ADDRESS,
            "seqno",
            "--arg",
            "int:1",
            "--stack-json",
            "[]",
        ])
        .is_err()
    );
}

#[test]
fn parses_contract_run_get_method_stack_cli_args() {
    let cli = Cli::try_parse_from([
        "tonutils",
        "contract",
        "run-get-method",
        "--address",
        RAW_ADDRESS,
        "--method",
        "seqno",
        "--arg",
        "int:1",
        "--arg",
        "null",
    ])
    .unwrap();

    match cli.command {
        Commands::Contract {
            command:
                ContractCommand::RunGetMethod {
                    args, stack_json, ..
                },
        } => {
            assert_eq!(args, vec!["int:1", "null"]);
            assert_eq!(stack_json, None);
        }
        _ => panic!("unexpected command"),
    }
}

#[test]
fn parses_contract_run_get_method_stack_json_cli_arg() {
    let cli = Cli::try_parse_from([
        "tonutils",
        "contract",
        "run-get-method",
        "--address",
        RAW_ADDRESS,
        "--method-id",
        "85143",
        "--stack-json",
        r#"[{"type":"int","value":"1"}]"#,
    ])
    .unwrap();

    match cli.command {
        Commands::Contract {
            command:
                ContractCommand::RunGetMethod {
                    args, stack_json, ..
                },
        } => {
            assert!(args.is_empty());
            assert_eq!(
                stack_json.as_deref(),
                Some(r#"[{"type":"int","value":"1"}]"#)
            );
        }
        _ => panic!("unexpected command"),
    }
}

#[test]
fn rejects_mixed_contract_run_get_method_stack_inputs() {
    assert!(
        Cli::try_parse_from([
            "tonutils",
            "contract",
            "run-get-method",
            "--address",
            RAW_ADDRESS,
            "--method",
            "seqno",
            "--arg",
            "null",
            "--stack-json",
            "[]",
        ])
        .is_err()
    );
}

#[test]
fn parses_stack_json_entries() {
    let boc = empty_boc_hex();
    let json = format!(
        r#"[
            {{"type":"null"}},
            {{"type":"int","value":"-5"}},
            {{"type":"cell","boc":"{boc}"}},
            {{"type":"slice","boc":"{boc}"}},
            {{"type":"tuple","entries":[{{"type":"int","value":"1"}}]}},
            {{"type":"list","entries":[{{"type":"null"}}]}},
            {{"type":"unsupported","raw":"0a0b"}}
        ]"#
    );
    let stack = parse_stack_json(&json).unwrap();

    assert_eq!(stack.entries().len(), 7);
    assert!(matches!(stack.entries()[0], TvmStackEntry::Null));
    assert_eq!(stack.entries()[1], TvmStackEntry::int(-5));
    assert!(matches!(stack.entries()[2], TvmStackEntry::Cell(_)));
    assert!(matches!(stack.entries()[3], TvmStackEntry::Slice(_)));
    assert!(matches!(stack.entries()[4], TvmStackEntry::Tuple(_)));
    assert!(matches!(stack.entries()[5], TvmStackEntry::List(_)));
    assert_eq!(stack.entries()[6], TvmStackEntry::Unsupported(vec![10, 11]));
}

#[test]
fn parses_extended_typed_stack_args() {
    let huge = "12345678901234567890123456789012345678901234567890";
    assert_eq!(
        parse_stack_arg(&format!("int:{huge}")).unwrap(),
        TvmStackEntry::int(BigInt::parse_bytes(huge.as_bytes(), 10).unwrap())
    );
    assert_eq!(
        parse_stack_arg("unsupported:0a0b").unwrap(),
        TvmStackEntry::Unsupported(vec![10, 11])
    );
    assert!(matches!(
        parse_stack_arg(r#"tuple:[{"type":"int","value":"1"}]"#).unwrap(),
        TvmStackEntry::Tuple(entries) if entries == vec![TvmStackEntry::int(1)]
    ));
    assert!(matches!(
        parse_stack_arg(r#"list:[{"type":"null"}]"#).unwrap(),
        TvmStackEntry::List(entries) if entries == vec![TvmStackEntry::Null]
    ));
}

#[test]
fn rejects_invalid_extended_typed_stack_args() {
    assert!(parse_stack_arg("unsupported:xx").is_err());
    assert!(parse_stack_arg(r#"tuple:{"type":"null"}"#).is_err());
    assert!(parse_stack_arg(r#"list:{"type":"null"}"#).is_err());
    assert!(parse_stack_arg("uint:1").is_err());
}

#[test]
fn rejects_invalid_stack_json() {
    for value in [
        r#"{"type":"null"}"#,
        r#"[{"type":"int","value":"not-a-number"}]"#,
        r#"[{"type":"cell","boc":"00"}]"#,
        r#"[{"type":"unsupported","raw":"xx"}]"#,
        r#"[{"type":"unknown"}]"#,
    ] {
        assert!(parse_stack_json(value).is_err(), "{value}");
    }
}
