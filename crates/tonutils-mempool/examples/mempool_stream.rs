use futures::StreamExt;
use tonutils_mempool::{MempoolConfig, MempoolEvent, MempoolScanner};
use tonutils_overlay::{OverlayId, PeerId, RoutingMetadata};
use tonutils_tlb::{
    CommonMsgInfo, Either, Grams, Message, MsgAddressExt, MsgAddressInt, TlbSerialize,
};
use tonutils_tvm::{Address, Builder, serialize_boc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scanner = MempoolScanner::new(MempoolConfig::default())?;
    let mut events = Box::pin(scanner.events());
    let routing = RoutingMetadata::new(OverlayId::from_name(b"local"), PeerId::from_bytes([0; 32]));

    let mut body = Builder::new();
    body.store_u32(0xfeed_beef)?;
    let message = Message {
        info: CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(Address::new(0, [0x22; 32])),
            import_fee: Grams::from(0),
        },
        init: None,
        body: Either::Left(body.build()?),
    };
    let raw_boc = serialize_boc(&message.to_cell()?, false)?;
    scanner.ingest(raw_boc, routing).await?;
    if let Some(MempoolEvent::ExternalMessage { hash, raw_boc, .. }) = events.next().await {
        println!("seen {hash:?} ({} bytes)", raw_boc.len());
    }
    Ok(())
}
