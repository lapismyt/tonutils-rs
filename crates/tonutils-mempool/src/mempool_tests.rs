use super::*;
use futures::StreamExt;

struct PendingSession {
    peer: PeerId,
}

impl OverlaySession for PendingSession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        Box::pin(std::future::pending())
    }

    fn send(&mut self, _payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

fn boc(body: u8) -> Vec<u8> {
    vec![0xb5, 0xee, 0x9c, 0x72, body]
}

fn config() -> MempoolConfig {
    MempoolConfig {
        validate_message: false,
        ..Default::default()
    }
}

#[tokio::test]
async fn publishes_once_for_duplicate_peers() {
    let scanner = MempoolScanner::new(config()).unwrap();
    let mut events = Box::pin(scanner.events());
    let routing = RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32]));
    let raw = boc(7);
    assert!(
        scanner
            .ingest(raw.clone(), routing.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert!(scanner.ingest(raw, routing).await.unwrap().is_none());
    assert!(matches!(
        events.next().await,
        Some(MempoolEvent::ExternalMessage { .. })
    ));
}

#[test]
fn event_exposes_lazy_message_without_decoding() {
    let event = MempoolEvent::ExternalMessage {
        hash: [3; 32],
        raw_boc: Arc::from([0xb5, 0xee, 0x9c, 0x72].as_slice()),
        destination: None,
        routing: RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32])),
        timestamp: SystemTime::now(),
    };
    let lazy = event.lazy_message().unwrap();
    assert_eq!(lazy.hash(), [3; 32]);
    assert_eq!(lazy.raw_boc().as_ref(), [0xb5, 0xee, 0x9c, 0x72]);
}

#[tokio::test]
async fn rejects_non_boc_fast() {
    let scanner = MempoolScanner::new(config()).unwrap();
    assert_eq!(
        scanner
            .ingest(
                vec![1, 2, 3, 4],
                RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32]))
            )
            .await,
        Err(MempoolError::InvalidBoc)
    );
}

#[tokio::test]
async fn builder_merges_and_deduplicates_explicit_seeds() {
    let seed = SeedPeer {
        peer: PeerId::from_bytes([9; 32]),
        address: "127.0.0.1:30303".into(),
    };
    let (scanner, manager, events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seeds([seed.clone(), seed])
        .start()
        .await
        .unwrap();

    scanner
        .ingest(
            vec![0xb5, 0xee, 0x9c, 0x72, 1],
            RoutingMetadata::new(
                OverlayId::from_name(b"mempool"),
                PeerId::from_bytes([9; 32]),
            ),
        )
        .await
        .unwrap();
    let mut events = Box::pin(events);
    assert!(matches!(
        events.next().await,
        Some(MempoolEvent::ExternalMessage { .. })
    ));
    manager.shutdown();
}

#[tokio::test]
async fn builder_requires_a_verified_bootstrap_source() {
    let result = MempoolScannerBuilder::new()
        .download_config(false)
        .start()
        .await;
    assert!(matches!(result, Err(MempoolError::NoBootstrapPeers)));
}

#[tokio::test]
async fn builder_connects_validated_peers_through_factory() {
    let peer = PeerId::from_bytes([5; 32]);
    let seed = SeedPeer {
        peer,
        address: "127.0.0.1:30303".into(),
    };
    let factory: OverlaySessionFactory = Arc::new(move |seed| {
        Box::pin(async move {
            assert_eq!(seed.peer, peer);
            Ok(Box::new(PendingSession { peer }) as Box<dyn OverlaySession>)
        })
    });
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(seed)
        .session_factory(factory)
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(manager.peer_count().await, 1);
    manager.shutdown();
}

#[tokio::test]
async fn dedup_capacity_is_global_across_shards() {
    let scanner = MempoolScanner::new(MempoolConfig {
        dedup_shards: 8,
        max_dedup_entries: 1,
        validate_message: false,
        ..Default::default()
    })
    .unwrap();
    let routing = RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32]));
    assert!(
        scanner
            .ingest(boc(1), routing.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert!(scanner.ingest(boc(2), routing).await.unwrap().is_some());
    assert!(scanner.dedup_entries.load(Ordering::Relaxed) <= 1);
}
