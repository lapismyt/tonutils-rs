use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{
    mempool::{MempoolConfig, MempoolError, MempoolEvent},
    tvm::{Builder, serialize_boc},
};

use super::{external_boc, new_server, next_external_event, routing, shutdown};

#[tokio::test]
async fn validates_external_message_and_preserves_shared_raw_bytes() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    let raw = external_boc(0xfeed_beef);
    let expected_hash: [u8; 32] = Sha256::digest(&raw).into();
    let mut events = Box::pin(server.events());

    assert_eq!(
        client.ingest(raw.clone(), routing()).await.unwrap(),
        Some(expected_hash)
    );
    let MempoolEvent::ExternalMessage {
        hash,
        raw_boc,
        destination,
        ..
    } = next_external_event(&mut events).await
    else {
        unreachable!("next_external_event only returns external messages");
    };
    assert_eq!(hash, expected_hash);
    assert_eq!(raw_boc.as_ref(), raw.as_slice());
    assert_eq!(destination, Some([0x42; 32]));
    let lazy = raw_boc.clone();
    assert_eq!(raw_boc.as_ptr(), lazy.as_ptr());
    assert_eq!(server.metrics().accepted, 1);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn rejects_invalid_input_before_dedup_and_counts_each_attempt() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    let bad = vec![1, 2, 3, 4];
    assert_eq!(
        client.ingest(bad.clone(), routing()).await,
        Err(MempoolError::InvalidBoc)
    );
    assert_eq!(
        client.ingest(bad, routing()).await,
        Err(MempoolError::InvalidBoc)
    );
    let metrics = server.metrics();
    assert_eq!(metrics.rejected, 2);
    assert_eq!(metrics.duplicates, 0);
    assert_eq!(metrics.accepted, 0);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn rejects_a_valid_boc_that_is_not_an_external_message() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    let mut cell = Builder::new();
    cell.store_u32(0x1234_5678).unwrap();
    let raw = serialize_boc(&cell.build().unwrap(), false).unwrap();
    assert_eq!(
        client.ingest(raw, routing()).await,
        Err(MempoolError::InvalidMessage)
    );
    assert_eq!(server.metrics().rejected, 1);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn deduplicates_until_the_ttl_expires() {
    let (server, client, overlay) = new_server(MempoolConfig {
        dedup_ttl: Duration::from_millis(5),
        ..MempoolConfig::default()
    })
    .await;
    let raw = external_boc(7);
    assert!(
        client
            .ingest(raw.clone(), routing())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        client
            .ingest(raw.clone(), routing())
            .await
            .unwrap()
            .is_none()
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(client.ingest(raw, routing()).await.unwrap().is_some());
    let metrics = server.metrics();
    assert_eq!(metrics.accepted, 2);
    assert_eq!(metrics.duplicates, 1);
    assert_eq!(metrics.rejected, 0);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn dedup_capacity_stays_bounded_across_shards() {
    let (server, client, overlay) = new_server(MempoolConfig {
        dedup_shards: 8,
        max_dedup_entries: 1,
        ..MempoolConfig::default()
    })
    .await;
    assert!(
        client
            .ingest(external_boc(1), routing())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        client
            .ingest(external_boc(2), routing())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(client.core.dedup.len(), 1);

    shutdown(server, overlay).await;
}

#[tokio::test]
async fn events_expose_lazy_message_without_decoding() {
    let raw = std::sync::Arc::<[u8]>::from([0xb5, 0xee, 0x9c, 0x72].as_slice());
    let event = MempoolEvent::ExternalMessage {
        hash: [3; 32],
        raw_boc: std::sync::Arc::clone(&raw),
        destination: None,
        routing: routing(),
        timestamp: std::time::SystemTime::now(),
    };
    let lazy = event.lazy_message().unwrap();
    assert_eq!(lazy.hash(), [3; 32]);
    assert_eq!(lazy.raw_boc().as_ptr(), raw.as_ptr());
    assert!(
        lazy.decode().is_err(),
        "the lazy view must decode on demand"
    );
}

#[tokio::test]
async fn callback_handler_receives_external_event() {
    let (server, client, overlay) = new_server(MempoolConfig::default()).await;
    let (sent, received) = tokio::sync::oneshot::channel();
    let handler_client = client.clone();
    let task = tokio::spawn(async move {
        let mut sent = Some(sent);
        handler_client
            .run_handler(move |event| {
                let sender = if matches!(event, MempoolEvent::ExternalMessage { .. }) {
                    sent.take()
                } else {
                    None
                };
                async move {
                    if let Some(sender) = sender {
                        let _ = sender.send(());
                    }
                }
            })
            .await;
    });
    client.ingest(external_boc(8), routing()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();

    shutdown(server, overlay).await;
}
