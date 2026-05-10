use num_bigint::BigUint;
use std::sync::Arc;
use tonutils::tlb::{
    CellRef, Either, Grams, MsgAddress, MsgAddressInt, Tlb, TlbDeserialize, TlbSerialize,
    VarUInteger,
};
use tonutils::tvm::{Address, Builder, Cell};

#[derive(Debug, Clone, PartialEq, Eq, Tlb)]
#[tlb(tag = "0x0f8a7ea5")]
struct JettonTransferMsg {
    query_id: u64,
    amount: VarUInteger<4>,
    destination: MsgAddress,
    response_destination: MsgAddress,
    custom_payload: Option<CellRef<Arc<Cell>>>,
    forward_ton_amount: Grams,
    forward_payload: Either<Arc<Cell>, CellRef<Arc<Cell>>>,
}

fn main() -> anyhow::Result<()> {
    let message = jetton_transfer_message()?;

    let cell = message.to_cell()?;
    let decoded = JettonTransferMsg::from_cell(cell.clone())?;
    assert_eq!(decoded, message);

    println!(
        "query_id={} amount={} destination={} custom_payload={} forward_ton_amount={} forward_payload={} root_hash={}",
        decoded.query_id,
        decoded.amount.0,
        address_label(&decoded.destination),
        decoded.custom_payload.is_some(),
        decoded.forward_ton_amount.0,
        forward_payload_label(&decoded.forward_payload),
        hex::encode(cell.hash())
    );
    Ok(())
}

fn jetton_transfer_message() -> anyhow::Result<JettonTransferMsg> {
    let forward_payload = {
        let mut builder = Builder::new();
        builder.store_u32(0)?;
        builder.store_bytes(b"derived jetton transfer")?;
        builder.build()?
    };

    Ok(JettonTransferMsg {
        query_id: 7,
        amount: VarUInteger(BigUint::from(42_u64)),
        destination: MsgAddress::Int(MsgAddressInt::std(Address::new(0, [0x11; 32]))),
        response_destination: MsgAddress::Int(MsgAddressInt::std(Address::new(0, [0x22; 32]))),
        custom_payload: None,
        forward_ton_amount: Grams::from(1_u64),
        forward_payload: Either::Left(forward_payload),
    })
}

fn address_label(address: &MsgAddress) -> String {
    match address {
        MsgAddress::Int(MsgAddressInt::Std { address, .. }) => {
            format!("{}:{}", address.workchain, hex::encode(address.hash_part))
        }
        other => format!("{other:?}"),
    }
}

fn forward_payload_label(payload: &Either<Arc<Cell>, CellRef<Arc<Cell>>>) -> &'static str {
    match payload {
        Either::Left(_) => "inline",
        Either::Right(_) => "reference",
    }
}
