use super::*;
use crate::jetton::{inline_forward_payload, referenced_forward_payload};
use crate::tlb::{Either, MsgAddress, MsgAddressExt, TlbDeserialize};
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
fn builds_and_decodes_nft_transfer_payload() {
    let custom = comment_cell("custom");
    let forward = comment_cell("forward");
    let payload = NftTransferPayload::new(1, address(1), address(2))
        .with_custom_payload(custom.clone())
        .with_forward_payload(777u64, referenced_forward_payload(forward.clone()));

    let decoded = exact_from_cell::<NftTransferPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 1);
    assert_eq!(decoded.new_owner, crate::jetton::std_address(address(1)));
    assert_eq!(
        decoded.response_destination,
        crate::jetton::std_address(address(2))
    );
    assert_eq!(decoded.custom_payload.unwrap().0.hash(), custom.hash());
    assert_eq!(decoded.forward_amount, BigUint::from(777u64));
    match decoded.forward_payload {
        Either::Right(cell_ref) => assert_eq!(cell_ref.0.hash(), forward.hash()),
        Either::Left(_) => panic!("expected referenced forward payload"),
    }
}

#[test]
fn builds_and_decodes_ownership_assigned_payload() {
    let forward = comment_cell("assigned");
    let payload =
        NftOwnershipAssignedPayload::new(2, address(3), inline_forward_payload(forward.clone()));

    let decoded =
        exact_from_cell::<NftOwnershipAssignedPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 2);
    assert_eq!(decoded.prev_owner, crate::jetton::std_address(address(3)));
    match decoded.forward_payload {
        Either::Left(cell) => assert_eq!(cell.hash(), forward.hash()),
        Either::Right(_) => panic!("expected inline forward payload"),
    }
}

#[test]
fn builds_and_decodes_report_static_data_payload() {
    let payload = NftReportStaticDataPayload::new(3, BigUint::from(123u64), Some(address(4)));
    let decoded =
        exact_from_cell::<NftReportStaticDataPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 3);
    assert_eq!(decoded.index, BigUint::from(123u64));
    assert_eq!(decoded.collection, crate::jetton::std_address(address(4)));
}

#[test]
fn report_static_data_supports_empty_collection_address() {
    let payload = NftReportStaticDataPayload::new(4, 0u64, None);
    let decoded =
        exact_from_cell::<NftReportStaticDataPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.collection, MsgAddress::Ext(MsgAddressExt::None));
}

#[test]
fn builds_and_decodes_report_royalty_params_payload() {
    let payload = NftReportRoyaltyParamsPayload::new(5, 11, 1000, address(5));
    let decoded =
        exact_from_cell::<NftReportRoyaltyParamsPayload>(payload.to_cell().unwrap()).unwrap();

    assert_eq!(decoded.query_id, 5);
    assert_eq!(decoded.numerator, 11);
    assert_eq!(decoded.denominator, 1000);
    assert_eq!(decoded.destination, crate::jetton::std_address(address(5)));
}

#[test]
fn query_only_nft_payloads_contain_op_and_query_id() {
    for (cell, op) in [
        (nft_excesses_payload(6).unwrap(), NFT_EXCESSES_OP),
        (
            nft_get_static_data_payload(6).unwrap(),
            NFT_GET_STATIC_DATA_OP,
        ),
        (
            nft_get_royalty_params_payload(6).unwrap(),
            NFT_GET_ROYALTY_PARAMS_OP,
        ),
    ] {
        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_u32().unwrap(), op);
        assert_eq!(slice.load_u64().unwrap(), 6);
        assert!(slice.is_empty());
    }
}

#[test]
fn rejects_wrong_nft_payload_opcode() {
    let cell = nft_excesses_payload(1).unwrap();
    let err = NftTransferPayload::from_cell(cell).unwrap_err();

    assert!(err.to_string().contains("operation code"));
}
