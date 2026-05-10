use num_bigint::BigUint;
use tonutils::tlb::{
    CommonMsgInfo, CurrencyCollection, Either, Grams, Message, MsgAddressInt, TlbDeserialize,
    TlbSerialize,
};
use tonutils::tvm::{Address, Builder};

fn main() -> anyhow::Result<()> {
    let inline_message = message(false)?;
    let inline_cell = inline_message.to_cell()?;
    let inline_decoded = Message::from_cell(inline_cell)?;

    let referenced_message = message(true)?;
    let referenced_cell = referenced_message.to_cell()?;
    let referenced_decoded = Message::from_cell(referenced_cell.clone())?;

    println!(
        "inline_body={} referenced_body={} value={} root_hash={}",
        matches!(inline_decoded.body, Either::Left(_)),
        matches!(referenced_decoded.body, Either::Right(_)),
        value_grams(&referenced_decoded),
        hex::encode(referenced_cell.hash())
    );

    Ok(())
}

fn message(referenced_body: bool) -> anyhow::Result<Message> {
    let body = {
        let mut builder = Builder::new();
        builder.store_u32(0xfeed_beef)?;
        builder.build()?
    };

    Ok(Message {
        info: CommonMsgInfo::Internal {
            ihr_disabled: true,
            bounce: true,
            bounced: false,
            src: MsgAddressInt::std(Address::new(0, [0x11; 32])),
            dest: MsgAddressInt::std(Address::new(0, [0x22; 32])),
            value: CurrencyCollection::grams(Grams::from(123_456_u64)),
            extra_flags: BigUint::from(0_u8),
            fwd_fee: Grams::from(1_u64),
            created_lt: 9,
            created_at: 1_700_000_000,
        },
        init: None,
        body: if referenced_body {
            Either::Right(body)
        } else {
            Either::Left(body)
        },
    })
}

fn value_grams(message: &Message) -> &BigUint {
    match &message.info {
        CommonMsgInfo::Internal { value, .. } => &value.grams.0,
        _ => unreachable!("fixture uses an internal message"),
    }
}
