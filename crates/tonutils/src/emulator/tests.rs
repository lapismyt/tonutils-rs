//! Offline tests for response parsing and configuration handling.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use serde_json::json;

use super::config::BlockchainConfig;
use super::engine::EmulatorEngine;
use super::error::EmulatorError;
use super::types::{parse_get_method_result, parse_message_result};
use crate::tvm::{Builder, VmStack, boc::boc_to_base64, boc::serialize_boc};

fn tiny_cell(byte: u8) -> std::sync::Arc<crate::tvm::Cell> {
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
