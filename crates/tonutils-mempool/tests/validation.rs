use std::time::Duration;

use futures::StreamExt;
use tonutils_mempool::{MempoolConfig, MempoolError, MempoolEvent, MempoolScanner};
use tonutils_overlay::{OverlayId, PeerId, RoutingMetadata};
use tonutils_tlb::{
    CommonMsgInfo, Either, Grams, Message, MsgAddressExt, MsgAddressInt, TlbSerialize,
};
use tonutils_tvm::{Address, Builder, serialize_boc};

fn external_boc(body: u32) -> Vec<u8> {
    let mut body_cell = Builder::new();
    body_cell.store_u32(body).unwrap();
    let message = Message {
        info: CommonMsgInfo::ExternalIn {
            src: MsgAddressExt::None,
            dest: MsgAddressInt::std(Address::new(0, [0x42; 32])),
            import_fee: Grams::from(0),
        },
        init: None,
        body: Either::Left(body_cell.build().unwrap()),
    };
    serialize_boc(&message.to_cell().unwrap(), false).unwrap()
}

fn routing() -> RoutingMetadata {
    RoutingMetadata::new(
        OverlayId::from_name(b"validation"),
        PeerId::from_bytes([1; 32]),
    )
}

#[tokio::test]
async fn validates_external_message_and_preserves_shared_raw_bytes() {
    let scanner = MempoolScanner::new(MempoolConfig::default()).unwrap();
    let raw = external_boc(0xfeed_beef);
    let mut events = Box::pin(scanner.events());
    scanner.ingest(raw.clone(), routing()).await.unwrap();
    let event = events.next().await.unwrap();
    let MempoolEvent::ExternalMessage {
        raw_boc,
        destination,
        ..
    } = event
    else {
        panic!("expected external message event");
    };
    assert_eq!(raw_boc.as_ref(), raw.as_slice());
    assert_eq!(destination, Some([0x42; 32]));
    let lazy = MempoolEvent::ExternalMessage {
        hash: [0; 32],
        raw_boc: raw_boc.clone(),
        destination: None,
        routing: routing(),
        timestamp: std::time::SystemTime::now(),
    }
    .lazy_message()
    .unwrap();
    assert_eq!(raw_boc.as_ptr(), lazy.raw_boc().as_ptr());
}

#[tokio::test]
async fn rejects_invalid_boc_and_expires_dedup_entries() {
    let scanner = MempoolScanner::new(MempoolConfig {
        validate_message: false,
        dedup_ttl: Duration::from_millis(1),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        scanner.ingest(vec![1, 2, 3, 4], routing()).await,
        Err(MempoolError::InvalidBoc)
    );
    let raw = vec![0xb5, 0xee, 0x9c, 0x72, 9];
    assert!(
        scanner
            .ingest(raw.clone(), routing())
            .await
            .unwrap()
            .is_some()
    );
    tokio::time::sleep(Duration::from_millis(3)).await;
    assert!(scanner.ingest(raw, routing()).await.unwrap().is_some());
    assert_eq!(scanner.metrics().rejected, 1);
}

#[tokio::test]
async fn rejects_a_valid_boc_that_is_not_an_external_message() {
    let scanner = MempoolScanner::new(MempoolConfig::default()).unwrap();
    let mut cell = Builder::new();
    cell.store_u32(0x1234_5678).unwrap();
    let raw = serialize_boc(&cell.build().unwrap(), false).unwrap();
    assert_eq!(
        scanner.ingest(raw, routing()).await,
        Err(MempoolError::InvalidMessage)
    );
}
