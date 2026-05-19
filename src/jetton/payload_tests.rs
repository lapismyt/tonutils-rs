use super::*;
use crate::tlb::{Either, TlbDeserialize};
use crate::tvm::{Address, Builder, Slice};
use num_bigint::BigUint;

fn address(seed: u8) -> Address {
    Address::new(0, [seed; 32])
}

fn comment_cell(text: &str) -> std::sync::Arc<crate::tvm::Cell> {
    let mut builder = Builder::new();
    builder.store_u32(0).unwrap();
    builder.store_bytes(text.as_bytes()).unwrap();
    builder.build().unwrap()
}

#[test]
fn builds_and_decodes_jetton_transfer_payload() {
    let custom = comment_cell("custom");
    let forward = comment_cell("hello");
    let payload = JettonTransferPayload::new(7, 123_456u64, address(1), address(2))
        .with_custom_payload(custom.clone())
        .with_forward_payload(1_000_000u64, referenced_forward_payload(forward.clone()));

    let cell = payload.to_cell().unwrap();
    let decoded = exact_from_cell::<JettonTransferPayload>(cell).unwrap();

    assert_eq!(decoded.query_id, 7);
    assert_eq!(decoded.amount, BigUint::from(123_456u64));
    assert_eq!(decoded.destination, std_address(address(1)));
    assert_eq!(decoded.response_destination, std_address(address(2)));
    assert_eq!(decoded.custom_payload.unwrap().0.hash(), custom.hash());
    assert_eq!(decoded.forward_ton_amount, BigUint::from(1_000_000u64));
    match decoded.forward_payload {
        Either::Right(cell_ref) => assert_eq!(cell_ref.0.hash(), forward.hash()),
        Either::Left(_) => panic!("expected referenced forward payload"),
    }
}

#[test]
fn builds_and_decodes_jetton_burn_payload() {
    let payload = JettonBurnPayload::new(8, 9_999u64, address(3));
    let decoded = exact_from_cell::<JettonBurnPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 8);
    assert_eq!(decoded.amount, BigUint::from(9_999u64));
    assert_eq!(decoded.response_destination, std_address(address(3)));
    assert!(decoded.custom_payload.is_none());
}

#[test]
fn builds_and_decodes_jetton_internal_transfer_payload() {
    let forward = comment_cell("inner");
    let payload = JettonInternalTransferPayload::new(9, 10u64, address(4), address(5))
        .with_forward_payload(11u64, inline_forward_payload(forward.clone()));

    let decoded =
        exact_from_cell::<JettonInternalTransferPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 9);
    assert_eq!(decoded.amount, BigUint::from(10u64));
    assert_eq!(decoded.from, std_address(address(4)));
    assert_eq!(decoded.response_address, std_address(address(5)));
    assert_eq!(decoded.forward_ton_amount, BigUint::from(11u64));
    match decoded.forward_payload {
        Either::Left(cell) => assert_eq!(cell.hash(), forward.hash()),
        Either::Right(_) => panic!("expected inline forward payload"),
    }
}

#[test]
fn excesses_payload_contains_op_and_query_id() {
    let cell = jetton_excesses_payload(42).unwrap();
    let mut slice = Slice::new(cell);

    assert_eq!(slice.load_u32().unwrap(), JETTON_EXCESSES_OP);
    assert_eq!(slice.load_u64().unwrap(), 42);
    assert!(slice.is_empty());
}

#[test]
fn rejects_wrong_jetton_payload_opcode() {
    let cell = jetton_excesses_payload(1).unwrap();
    let err = JettonTransferPayload::from_cell(cell).unwrap_err();

    assert!(err.to_string().contains("operation code"));
}
