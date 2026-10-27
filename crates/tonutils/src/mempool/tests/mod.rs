mod common;
mod live;
mod live_probe;
mod live_quic;
mod seed_scanner;
mod udp_session;
mod validation;

use std::{
    collections::HashSet,
    sync::{Arc, atomic::Ordering},
    time::{Duration, SystemTime},
};

use crate::{
    adnl::AdnlKeyPair,
    mempool::{
        MempoolConfig, MempoolError, MempoolEvent, MempoolScanner, MempoolScannerBuilder,
        OverlaySessionFactory, PEER_STORE_VERSION, PeerQualityConfig, PeerStore, PeerStoreFile,
        StoredPeer,
        peer_growth::PeerGrowthTask,
        peer_store::{SOURCE_GROWTH, STATUS_CONNECTED},
    },
    overlay::{
        OverlayConfig, OverlayId, OverlayPacket, OverlaySession, PeerId, PeerManager, PeerStatus,
        ReconnectFactory, RoutingMetadata, SeedDiscoveryLookup, SeedPeer,
    },
};

use futures::{StreamExt, future::BoxFuture};

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

#[tokio::test]
async fn callback_handler_receives_external_event() {
    let scanner = Arc::new(MempoolScanner::new(config()).unwrap());
    let (sent, received) = tokio::sync::oneshot::channel();
    let handler_scanner = scanner.clone();
    let task = tokio::spawn(async move {
        let mut sent = Some(sent);
        handler_scanner
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
    scanner
        .ingest(
            boc(8),
            RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32])),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();
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
        .bootstrap_timeout(Duration::ZERO)
        .seeds([seed.clone(), seed])
        .start()
        .await
        .unwrap();

    scanner
        .ingest(
            vec![0xb5, 0xee, 0x9c, 0x72, 1],
            RoutingMetadata::new(
                OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
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
async fn seed_only_mode_ignores_global_discovery_sources() {
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let seed = SeedPeer::from_public_key(key.public_key.to_bytes(), "127.0.0.1:30303");
    let result = MempoolScannerBuilder::new()
        .global_config_json("not json")
        .seed(seed)
        .bootstrap_timeout(Duration::ZERO)
        .native_udp_seeds_only("0.0.0.0:0".parse().unwrap(), key, None)
        .start()
        .await;
    assert!(result.is_ok());
    if let Ok((_, manager, _)) = result {
        manager.shutdown_wait().await;
    }
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
async fn builder_keeps_bootstrap_seeds_out_of_membership_when_discovery_succeeds() {
    let seed_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let discovered_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let seed = SeedPeer::from_public_key(seed_key.public_key.to_bytes(), "127.0.0.1:30303");
    let discovered =
        SeedPeer::from_public_key(discovered_key.public_key.to_bytes(), "127.0.0.1:30304");
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let lookup: SeedDiscoveryLookup = Arc::new(move |_| {
        let discovered = discovered.clone();
        Box::pin(async move { vec![discovered] })
    });
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(seed)
        .seed_discovery_lookup(lookup)
        .session_factory(factory)
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1)).await;
    // Bootstrap seeds are DHT contacts: once discovery returned members,
    // no session is registered for the seed set (TODO item 411).
    assert_eq!(manager.peer_count().await, 1);
    manager.shutdown();
}

#[tokio::test]
async fn builder_falls_back_to_bootstrap_seeds_when_discovery_finds_nothing() {
    let seed_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let seed = SeedPeer::from_public_key(seed_key.public_key.to_bytes(), "127.0.0.1:30303");
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let lookup: SeedDiscoveryLookup = Arc::new(move |_| Box::pin(async move { Vec::new() }));
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(seed)
        .seed_discovery_lookup(lookup)
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

#[tokio::test]
async fn concurrent_ingest_publishes_one_copy() {
    let scanner = Arc::new(MempoolScanner::new(config()).unwrap());
    let routing = RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32]));
    let first = scanner.ingest(boc(7), routing.clone());
    let second = scanner.ingest(boc(7), routing);
    let (first, second) = tokio::join!(first, second);
    let accepted = [first.unwrap(), second.unwrap()]
        .into_iter()
        .flatten()
        .count();
    assert_eq!(accepted, 1);
    assert_eq!(scanner.metrics().duplicates, 1);
    assert_eq!(scanner.dedup_entries.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn amortized_purge_expires_stale_entries_exactly() {
    let scanner = MempoolScanner::new(MempoolConfig {
        dedup_shards: 1,
        dedup_ttl: Duration::from_millis(5),
        validate_message: false,
        ..MempoolConfig::default()
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
    assert!(
        scanner
            .ingest(boc(2), routing.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(scanner.dedup_entries.load(Ordering::Relaxed), 2);
    tokio::time::sleep(Duration::from_millis(20)).await;
    // The next new message runs the amortized purge before inserting, so
    // both stale entries drop while the fresh one stays accounted exactly.
    assert!(scanner.ingest(boc(3), routing).await.unwrap().is_some());
    assert_eq!(scanner.dedup_entries.load(Ordering::Relaxed), 1);
    assert_eq!(scanner.metrics().duplicates, 0);
}

#[tokio::test]
async fn overlay_receiver_stops_with_manager_shutdown() {
    let scanner = Arc::new(MempoolScanner::new(config()).unwrap());
    let manager = PeerManager::new(OverlayConfig::default()).unwrap();
    let receiver =
        scanner.spawn_overlay_receiver_with_shutdown(manager.pool(), manager.subscribe_shutdown());
    manager.shutdown_wait().await;
    receiver.await.unwrap();
}

/// The N receiver workers drain the pool queue exactly once each: every
/// injected packet is counted and processed a single time, with no loss
/// and no double-ingest across workers.
#[tokio::test]
async fn receiver_drains_pool_packets_across_workers() {
    const PACKETS: u8 = 32;

    let scanner = Arc::new(MempoolScanner::new(config()).unwrap());
    let manager = PeerManager::new(OverlayConfig::default()).unwrap();
    let pool = manager.pool();
    let receiver = scanner
        .clone()
        .spawn_overlay_receiver_with_shutdown(pool.clone(), manager.subscribe_shutdown());

    let peer = PeerId::from_bytes([1; 32]);
    pool.register_peer(peer).await;
    let routing = RoutingMetadata::new(OverlayId::from_name(b"test"), peer);
    for byte in 0..PACKETS {
        pool.ingest(OverlayPacket {
            payload: Arc::from([byte; 4].as_slice()),
            routing: routing.clone(),
        })
        .await
        .unwrap();
    }

    // The injected payloads are not BoCs, so each one must land in the
    // invalid-packet path exactly once - processed twice would show up
    // as more warnings than packets.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let metrics = scanner.metrics();
        if metrics.invalid_warnings >= PACKETS as u64 {
            assert_eq!(metrics.overlay_packets, PACKETS as u64);
            assert_eq!(metrics.invalid_warnings, PACKETS as u64);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workers did not drain the queue in time: {metrics:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    manager.shutdown_wait().await;
    receiver.await.unwrap();
}

/// A self-submitted message must not block on a full event queue: the
/// local echo is dropped with a warning while the dedup entry and the
/// broadcast stay intact.
#[tokio::test]
async fn local_echo_drops_rather_than_backing_up_a_full_event_queue() {
    let scanner = Arc::new(
        MempoolScanner::new(MempoolConfig {
            event_queue_capacity: 1,
            validate_message: false,
            ..Default::default()
        })
        .unwrap(),
    );
    // Fill the one-slot event queue with an event the scanner produced
    // itself, and leave it undrained.
    scanner
        .peer_status(PeerStatus::Disconnected {
            peer: PeerId::from_bytes([3; 32]),
        })
        .await
        .unwrap();
    let hash = tokio::time::timeout(Duration::from_secs(2), scanner.send_external(boc(1)))
        .await
        .expect("a local send must not block on a full event queue")
        .unwrap();
    let metrics = scanner.metrics();
    assert_eq!(metrics.accepted, 1, "the message itself is accepted");
    assert_eq!(scanner.dedup_entries.load(Ordering::Relaxed), 1);
    // The dedup entry survived the dropped echo: the repeat is rejected
    // instead of re-broadcast.
    assert_eq!(scanner.send_external(boc(1)).await.unwrap(), hash);
    assert_eq!(scanner.metrics().accepted, 1);
    // Only the foreign event was ever queued; the echo never surfaced.
    let mut events = std::pin::pin!(scanner.events());
    assert!(matches!(
        events.next().await,
        Some(MempoolEvent::PeerStatus(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.next())
            .await
            .is_err(),
        "the dropped local echo must not surface as an event"
    );
}

/// A disconnect starts a growth round right away instead of waiting out
/// the periodic interval, and a burst of disconnects collapses into the
/// round that already ran.
#[tokio::test]
async fn disconnect_triggers_an_immediate_growth_round() {
    let scanner = Arc::new(MempoolScanner::new(config()).unwrap());
    let manager = PeerManager::new(OverlayConfig::default()).unwrap();
    let rounds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let growth_rounds = Arc::clone(&rounds);
    let growth: SeedDiscoveryLookup = Arc::new(move |_seeds| {
        growth_rounds.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Vec::new() })
    });
    let known = Arc::new(tokio::sync::RwLock::new(vec![SeedPeer {
        peer: PeerId::from_bytes([9; 32]),
        address: "127.0.0.1:30303".into(),
    }]));
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let reconnect: ReconnectFactory =
        Arc::new(move |_peer| Box::pin(async { Err("no reconnect in this test".to_owned()) }));
    PeerGrowthTask {
        manager: manager.clone(),
        growth,
        known,
        factory,
        reconnect,
        attempts: 0,
        backoff: Duration::ZERO,
        max_peers: 8,
        pinned: HashSet::new(),
        growth_interval: Duration::from_secs(10),
        event_tx: scanner.event_tx.clone(),
        store: None,
    }
    .spawn();

    // Losing a member must start discovery without waiting out the
    // periodic interval (10 s), which no test would sit through.
    let member = PeerId::from_bytes([1; 32]);
    manager.pool().register_peer(member).await;
    manager.pool().unregister_peer(member).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while rounds.load(Ordering::SeqCst) == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no growth round after a disconnect"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // A second disconnect inside the guard window collapses into the
    // round that already ran instead of starting another one.
    let other = PeerId::from_bytes([2; 32]);
    manager.pool().register_peer(other).await;
    manager.pool().unregister_peer(other).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rounds.load(Ordering::SeqCst),
        1,
        "a burst of disconnects must not storm discovery rounds"
    );
    manager.shutdown();
}

#[tokio::test]
async fn pinned_peers_honor_the_isolated_connection_limit() {
    let kept = PeerId::from_bytes([3; 32]);
    let dropped = PeerId::from_bytes([4; 32]);
    let seed_peer = PeerId::from_bytes([6; 32]);
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(SeedPeer {
            peer: seed_peer,
            address: "127.0.0.1:30303".into(),
        })
        .pinned_peers([
            SeedPeer {
                peer: kept,
                address: "127.0.0.1:30301".into(),
            },
            SeedPeer {
                peer: dropped,
                address: "127.0.0.1:30302".into(),
            },
        ])
        .pinned_max_peers(Some(1))
        .session_factory(factory)
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    let peers = manager.peers().await;
    assert_eq!(peers.len(), 2, "one pinned plus one seed session connects");
    assert!(peers.contains(&kept) && peers.contains(&seed_peer));
    assert!(
        !peers.contains(&dropped),
        "pinned entries above the isolated limit must be dropped"
    );
    manager.shutdown_wait().await;
}

#[tokio::test]
async fn quality_eviction_drops_silent_peers_and_keeps_pinned() {
    let pinned = PeerId::from_bytes([1; 32]);
    let silent = PeerId::from_bytes([2; 32]);
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let quality = PeerQualityConfig {
        late_threshold: Duration::from_millis(10),
        miss_window: Duration::from_millis(10),
        min_observations: 1,
        sweep_interval: Duration::from_millis(10),
        churn_interval: Duration::from_millis(20),
        ..PeerQualityConfig::default()
    };
    let (scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .pinned_peers([SeedPeer {
            peer: pinned,
            address: "127.0.0.1:30301".into(),
        }])
        .seed(SeedPeer {
            peer: silent,
            address: "127.0.0.1:30302".into(),
        })
        .overlay_max_peers(0)
        .quality_eviction(true)
        .peer_quality(quality)
        .session_factory(factory)
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        manager.peer_count().await,
        2,
        "pinned and seed sessions must bootstrap"
    );
    // The first-seen delivery credits the pinned peer; the silent peer sits
    // in the connected snapshot, misses the window, and becomes the only
    // eviction candidate over the zero managed cap.
    scanner
        .ingest(
            boc(3),
            RoutingMetadata::new(OverlayId::MAINNET_BASECHAIN_OVERLAY_ID, pinned),
        )
        .await
        .unwrap();
    let mut evicted = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if manager.peer_count().await == 1 {
            evicted = true;
            break;
        }
    }
    assert!(evicted, "the silent managed peer must be evicted");
    assert_eq!(
        manager.peers().await,
        vec![pinned],
        "the pinned peer must survive quality churn"
    );
    manager.shutdown_wait().await;
}

/// Unique per-test store file; always starts absent.
fn store_path(tag: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "tonutils-mempool-store-{}-{tag}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}.tmp", path.display()));
    path
}

fn write_store(path: &std::path::Path, peers: Vec<StoredPeer>) {
    let file = PeerStoreFile {
        version: PEER_STORE_VERSION,
        network: "mainnet".into(),
        overlay: hex::encode(OverlayId::MAINNET_BASECHAIN_OVERLAY_ID.as_bytes()),
        saved_at: 1_700_000_000,
        peers,
    };
    std::fs::write(path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
}

fn stored(id: u8, address: &str, priority: f64) -> StoredPeer {
    StoredPeer {
        id: PeerId::from_bytes([id; 32]),
        address: address.to_owned(),
        last_seen: 1_700_000_000,
        source: SOURCE_GROWTH.to_owned(),
        status: STATUS_CONNECTED.to_owned(),
        priority,
        udp: None,
        quic: None,
    }
}

fn accepting_factory() -> OverlaySessionFactory {
    Arc::new(move |candidate| {
        Box::pin(async move {
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    })
}

fn counting_lookup(counter: Arc<std::sync::atomic::AtomicUsize>) -> SeedDiscoveryLookup {
    Arc::new(move |_seeds| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { vec![SeedPeer::from_public_key([9; 32], "127.0.0.1:30304")] })
    })
}

/// A warm store joins the bootstrap dial set and the blocking start-time
/// discovery lookup must not run at all (the background task is parked far
/// outside the test window).
#[tokio::test]
async fn warm_peer_store_skips_the_blocking_discovery() {
    let path = store_path("warm");
    write_store(
        &path,
        vec![
            stored(1, "127.0.0.1:30301", 0.9),
            stored(2, "127.0.0.1:30302", 0.4),
        ],
    );
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(SeedPeer {
            peer: PeerId::from_bytes([7; 32]),
            address: "127.0.0.1:30399".into(),
        })
        .seed_discovery_lookup(counting_lookup(Arc::clone(&calls)))
        .session_factory(accepting_factory())
        .peer_store(&path)
        .peer_store_discovery_interval(Duration::from_secs(3600))
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        manager.peers().await.len(),
        2,
        "both stored peers must be dialed"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a warm store must skip the blocking startup discovery"
    );
    manager.shutdown_wait().await;
    let _ = std::fs::remove_file(&path);
}

/// A cold store keeps the legacy behavior: the blocking discovery runs once
/// at startup and its members form the session set.
#[tokio::test]
async fn cold_peer_store_keeps_the_legacy_startup_discovery() {
    let path = store_path("cold");
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(SeedPeer {
            peer: PeerId::from_bytes([7; 32]),
            address: "127.0.0.1:30399".into(),
        })
        .seed_discovery_lookup(counting_lookup(Arc::clone(&calls)))
        .session_factory(accepting_factory())
        .peer_store(&path)
        .peer_store_discovery_interval(Duration::from_secs(3600))
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        manager.peers().await,
        vec![SeedPeer::from_public_key([9; 32], "127.0.0.1:30304").peer],
        "the discovered member, not the DHT seed, must be registered"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    manager.shutdown_wait().await;
    let _ = std::fs::remove_file(&path);
}

/// When every warm-store dial fails, startup runs the skipped discovery once
/// as a fallback instead of failing outright.
#[tokio::test]
async fn warm_store_total_failure_falls_back_to_discovery() {
    let path = store_path("fallback");
    write_store(&path, vec![stored(1, "127.0.0.1:30311", 0.9)]);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let factory: OverlaySessionFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            if candidate.address == "127.0.0.1:30311" {
                return Err("stale stored address".to_owned());
            }
            Ok(Box::new(PendingSession {
                peer: candidate.peer,
            }) as Box<dyn OverlaySession>)
        })
    });
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(SeedPeer {
            peer: PeerId::from_bytes([7; 32]),
            address: "127.0.0.1:30399".into(),
        })
        .seed_discovery_lookup(counting_lookup(Arc::clone(&calls)))
        .session_factory(factory)
        .peer_store(&path)
        .peer_store_discovery_interval(Duration::from_secs(3600))
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the skipped discovery must run as the warm-store fallback"
    );
    assert_eq!(
        manager.peers().await,
        vec![SeedPeer::from_public_key([9; 32], "127.0.0.1:30304").peer],
        "the fallback discovery member must be registered"
    );
    manager.shutdown_wait().await;
    let _ = std::fs::remove_file(&path);
}

/// The store file exists after startup and still exists - with the run's
/// peers - after shutdown, which is the on-disk contract CI relies on.
#[tokio::test]
async fn shutdown_leaves_the_store_file_behind() {
    let path = store_path("shutdown");
    write_store(
        &path,
        vec![
            stored(1, "127.0.0.1:30301", 0.9),
            stored(2, "127.0.0.1:30302", 0.4),
        ],
    );
    let (_scanner, manager, _events) = MempoolScannerBuilder::new()
        .download_config(false)
        .config(config())
        .seed(SeedPeer {
            peer: PeerId::from_bytes([7; 32]),
            address: "127.0.0.1:30399".into(),
        })
        .seed_discovery_lookup(counting_lookup(Arc::new(
            std::sync::atomic::AtomicUsize::new(0),
        )))
        .session_factory(accepting_factory())
        .peer_store(&path)
        .peer_store_save_interval(Duration::from_secs(3600))
        .peer_store_discovery_interval(Duration::from_secs(3600))
        .start()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    manager.shutdown_wait().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reloaded = PeerStore::load(
        &path,
        "mainnet",
        &hex::encode(OverlayId::MAINNET_BASECHAIN_OVERLAY_ID.as_bytes()),
    );
    let seeds = reloaded.seeds().await;
    assert_eq!(seeds.len(), 2, "both persisted peers survive the run");
    assert!(
        seeds
            .iter()
            .any(|seed| seed.peer == PeerId::from_bytes([1; 32])),
        "the stored peer set must not be replaced by the run"
    );
    let _ = std::fs::remove_file(&path);
}
