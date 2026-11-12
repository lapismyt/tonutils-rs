//! Offline tests for response parsing and configuration handling.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use num_bigint::BigInt;
use serde_json::json;

use super::config::BlockchainConfig;
use super::engine::{EmulatorEngine, EmulatorLogLevel};
use super::error::EmulatorError;
use super::tvm::TvmEmulator;
use super::types::{parse_get_method_result, parse_message_result};
use crate::tvm::{
    Builder, Cell, VmStack, VmStackValue, boc::boc_to_base64, boc::hex_to_boc, boc::serialize_boc,
};

fn tiny_cell(byte: u8) -> Arc<Cell> {
    let mut builder = Builder::new();
    builder.store_u8(byte).unwrap();
    builder.build().unwrap()
}

#[test]
fn parses_get_method_success_with_string_gas() {
    let stack = VmStack::default();
    let stack_boc = B64_STANDARD.encode(stack.to_boc().unwrap());
    let value = json!({
        "success": true,
        "stack": stack_boc,
        "gas_used": "1212",
        "vm_exit_code": 0,
        "vm_log": "execute DUP",
        "missing_library": null
    });

    let result = parse_get_method_result(&value).unwrap();
    assert_eq!(result.stack, stack);
    assert_eq!(result.gas_used, 1212);
    assert_eq!(result.vm_exit_code, 0);
    assert_eq!(result.vm_log, "execute DUP");
    assert_eq!(result.missing_library, None);
}

#[test]
fn parses_get_method_failure_details() {
    let value = json!({
        "success": false,
        "error": "External message not accepted by smart contract",
        "external_not_accepted": true,
        "vm_exit_code": 9,
        "vm_log": "exit code 9",
        "elapsed_time": 0.25
    });

    let error = parse_get_method_result(&value).unwrap_err();
    assert!(error.external_not_accepted());
    assert_eq!(error.vm_exit_code(), Some(9));
    assert_eq!(error.vm_log(), Some("exit code 9"));
    let text = error.to_string();
    assert!(text.contains("External message not accepted"));
}

#[test]
fn parses_missing_library_hash() {
    let stack_boc = B64_STANDARD.encode(VmStack::default().to_boc().unwrap());
    let hash = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    let value = json!({
        "success": true,
        "stack": stack_boc,
        "gas_used": 1,
        "vm_exit_code": 0,
        "vm_log": "",
        "missing_library": hash
    });

    let result = parse_get_method_result(&value).unwrap();
    assert_eq!(
        result.missing_library,
        Some([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
            0xcc, 0xdd, 0xee, 0xff,
        ])
    );
}

#[test]
fn parses_message_success() {
    let code = tiny_cell(0x11);
    let data = tiny_cell(0x22);
    let value = json!({
        "success": true,
        "new_code": boc_to_base64(&code, false).unwrap(),
        "new_data": boc_to_base64(&data, false).unwrap(),
        "accepted": true,
        "vm_exit_code": 0,
        "vm_log": "ok",
        "gas_used": "42",
        "actions": null,
        "missing_library": null
    });

    let result = parse_message_result(&value).unwrap();
    assert_eq!(result.new_code, code);
    assert_eq!(result.new_data, data);
    assert!(result.accepted);
    assert_eq!(result.gas_used, 42);
    assert_eq!(result.actions, None);
}

#[test]
fn rejects_malformed_string_gas() {
    let stack_boc = B64_STANDARD.encode(VmStack::default().to_boc().unwrap());
    let value = json!({
        "success": true,
        "stack": stack_boc,
        "gas_used": "not-a-number",
        "vm_exit_code": 0,
        "vm_log": "",
        "missing_library": null
    });

    assert!(matches!(
        parse_get_method_result(&value),
        Err(EmulatorError::InvalidResponse(_))
    ));
}

#[test]
fn config_accepts_raw_and_base64_boc() {
    let cell = tiny_cell(0x33);
    let raw = serialize_boc(&cell, false).unwrap();
    let encoded = B64_STANDARD.encode(&raw);

    let from_raw = BlockchainConfig::from_bytes(&raw).unwrap();
    let from_text = BlockchainConfig::from_bytes(encoded.as_bytes()).unwrap();
    let from_direct = BlockchainConfig::from_dictionary_boc(&raw).unwrap();

    assert_eq!(from_raw, from_direct);
    assert_eq!(from_text, from_direct);
    assert_eq!(from_direct.dictionary(), &cell);
}

#[test]
fn missing_library_path_is_reported() {
    let path = "/definitely/not/present/libemulator.so";
    let error = EmulatorEngine::open(path).unwrap_err();
    assert!(matches!(error, EmulatorError::LibraryOpen(_)));
    assert!(error.to_string().contains("libemulator"));
}

// Live tests. They are ignored by default because `libemulator` is a native
// runtime dependency. Run them with a local library:
//
// ```text
// TONUTILS_EMULATOR_LIB=/path/to/libemulator.so \
//     cargo test -p tonutils --features emulator -- --ignored
// ```
//
// CI downloads the released library and runs these exact tests.

// Wallet v4r2 code and data from the public TON network. They are static
// fixtures: the get-method outputs below are deterministic for this cell pair.
const WALLET_V4R2_CODE: &str = "b5ee9c72410214010002d4000114ff00f4a413f4bcf2c80b010201200203020148040504f8f28308d71820d31fd31fd31f02f823bbf264ed44d0d31fd31fd3fff404d15143baf2a15151baf2a205f901541064f910f2a3f80024a4c8cb1f5240cb1f5230cbff5210f400c9ed54f80f01d30721c0009f6c519320d74a96d307d402fb00e830e021c001e30021c002e30001c0039130e30d03a4c8cb1f12cb1fcbff1011121302e6d001d0d3032171b0925f04e022d749c120925f04e002d31f218210706c7567bd22821064737472bdb0925f05e003fa403020fa4401c8ca07cbffc9d0ed44d0810140d721f404305c810108f40a6fa131b3925f07e005d33fc8258210706c7567ba923830e30d03821064737472ba925f06e30d06070201200809007801fa00f40430f8276f2230500aa121bef2e0508210706c7567831eb17080185004cb0526cf1658fa0219f400cb6917cb1f5260cb3f20c98040fb0006008a5004810108f45930ed44d0810140d720c801cf16f400c9ed540172b08e23821064737472831eb17080185005cb055003cf1623fa0213cb6acb1fcb3fc98040fb00925f03e20201200a0b0059bd242b6f6a2684080a06b90fa0218470d4080847a4937d29910ce6903e9ff9837812801b7810148987159f31840201580c0d0011b8c97ed44d0d70b1f8003db29dfb513420405035c87d010c00b23281f2fff274006040423d029be84c600201200e0f0019adce76a26840206b90eb85ffc00019af1df6a26840106b90eb858fc0006ed207fa00d4d422f90005c8ca0715cbffc9d077748018c8cb05cb0222cf165005fa0214cb6b12ccccc973fb00c84014810108f451f2a7020070810108d718fa00d33fc8542047810108f451f2a782106e6f746570748018c8cb05cb025006cf165004fa0214cb6a12cb1fcb3fc973fb0002006c810108d718fa00d33f305224810108f459f2a782106473747270748018c8cb05cb025005cf165003fa0213cb6acb1f12cb3fc973fb00000af400c9ed54696225e5";
const WALLET_V4R2_DATA: &str = "b5ee9c7201010101002b0000510000004629a9a31722f1d146dcc2a181f593205cfdcc7f1b845c15947dfe7ee1a9152175f096f89440";

fn wallet_v4r2() -> (Arc<Cell>, Arc<Cell>) {
    let code = hex_to_boc(WALLET_V4R2_CODE).expect("wallet code fixture must decode");
    let data = hex_to_boc(WALLET_V4R2_DATA).expect("wallet data fixture must decode");
    (code, data)
}

fn live_engine() -> Arc<EmulatorEngine> {
    EmulatorEngine::open_default()
        .expect("libemulator must be available via TONUTILS_EMULATOR_LIB, emulator-download, or the loader path")
}

fn assert_single_int(stack: &VmStack, expected: &str) {
    assert_eq!(stack.len(), 1, "expected one stack value, got {stack:?}");
    let expected = expected
        .parse::<BigInt>()
        .expect("expected value must be a valid decimal integer");
    match &stack.values()[0] {
        VmStackValue::Int(value) => assert_eq!(value, &expected),
        other => panic!("expected an integer stack value, got {other:?}"),
    }
}

#[test]
#[ignore = "requires libemulator; set TONUTILS_EMULATOR_LIB or enable emulator-download"]
fn live_engine_reports_version_and_accepts_verbosity() {
    let engine = live_engine();
    let version = engine.version().expect("version must be returned");
    assert!(!version.trim().is_empty(), "version must not be empty");
    engine
        .set_verbosity(EmulatorLogLevel::Error)
        .expect("verbosity setter must accept a valid level");
}

#[test]
#[ignore = "requires libemulator; set TONUTILS_EMULATOR_LIB or enable emulator-download"]
fn live_wallet_v4r2_seqno_get_method() {
    let (code, data) = wallet_v4r2();
    let mut emulator = TvmEmulator::builder(code, data)
        .engine(live_engine())
        .build()
        .expect("TVM emulator must be created");
    let result = emulator
        .run_get_method_by_name("seqno", &VmStack::default())
        .expect("seqno get-method must succeed");
    assert_eq!(result.vm_exit_code, 0, "vm log: {}", result.vm_log);
    assert_single_int(&result.stack, "70");
}

#[test]
#[ignore = "requires libemulator; set TONUTILS_EMULATOR_LIB or enable emulator-download"]
fn live_wallet_v4r2_public_key_get_method() {
    let (code, data) = wallet_v4r2();
    let mut emulator = TvmEmulator::builder(code, data)
        .engine(live_engine())
        .build()
        .expect("TVM emulator must be created");
    let result = emulator
        .run_get_method_by_name("get_public_key", &VmStack::default())
        .expect("get_public_key get-method must succeed");
    assert_eq!(result.vm_exit_code, 0, "vm log: {}", result.vm_log);
    assert_single_int(
        &result.stack,
        "15805891369874164427404339195218829417759249195229050328701160893742440708244",
    );
}

#[test]
#[ignore = "requires libemulator; set TONUTILS_EMULATOR_LIB or enable emulator-download"]
fn live_wallet_v4r2_internal_message_is_accepted() {
    let (code, data) = wallet_v4r2();
    let mut emulator = TvmEmulator::builder(code, data)
        .engine(live_engine())
        .build()
        .expect("TVM emulator must be created");
    let empty_body = Arc::new(Cell::empty());
    let result = emulator
        .send_internal_message(&empty_body, 1)
        .expect("internal message emulation must succeed");
    assert!(result.accepted, "wallet must accept the internal message");
    assert_eq!(result.vm_exit_code, 0);
}
