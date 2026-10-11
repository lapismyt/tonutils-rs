use super::*;
use ed25519_dalek::{Signer, SigningKey};
use futures::future::BoxFuture;

struct FlakySession {
    peer: PeerId,
    fail: bool,
}

impl OverlaySession for FlakySession {
    fn peer_id(&self) -> PeerId {
        self.peer
    }

    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
        if self.fail {
            self.fail = false;
            Box::pin(async { Err("synthetic failure".to_owned()) })
        } else {
            Box::pin(std::future::pending())
        }
    }

    fn send(&mut self, _payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn rejects_unknown_and_oversized_packets() {
    let pool = OverlayPeerPool::new(OverlayConfig {
        max_packet_size: 2,
        ..Default::default()
    })
    .unwrap();
    let packet = OverlayPacket {
        payload: Arc::from([1, 2, 3].as_slice()),
        routing: RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32])),
    };
    assert!(matches!(
        pool.ingest(packet).await,
        Err(OverlayError::PacketTooLarge)
    ));
    pool.register_peer(PeerId::from_bytes([1; 32])).await;
    let packet = OverlayPacket {
        payload: Arc::from([1].as_slice()),
        routing: RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32])),
    };
    pool.ingest(packet).await.unwrap();
    assert_eq!(pool.next_packet().await.unwrap().payload.as_ref(), [1]);
}

#[test]
fn discovery_rejects_bad_records_and_falls_back_to_seeds() {
    let overlay = OverlayId::from_name(b"mainnet");
    let key = SigningKey::from_bytes(&[7; 32]);
    let peer = PeerId::from_bytes([8; 32]);
    let mut record = DiscoveryRecord {
        overlay,
        peer,
        node_key: key.verifying_key().to_bytes(),
        address: "127.0.0.1:30303".to_owned(),
        signature: [0; 64],
    };
    record.signature = key.sign(&record.signed_bytes()).to_bytes();
    assert!(record.verify());

    let config = DiscoveryConfig {
        overlay,
        seeds: vec![SeedPeer {
            peer: PeerId::from_bytes([9; 32]),
            address: "127.0.0.1:30304".to_owned(),
        }],
        ..Default::default()
    };
    assert_eq!(select_discovery_peers(&config, [record]).len(), 1);
    assert_eq!(select_discovery_peers(&config, []).len(), 1);
}

#[tokio::test]
async fn discovery_falls_back_after_lookup_timeout() {
    let overlay = OverlayId::from_name(b"testnet");
    let config = DiscoveryConfig {
        overlay,
        seeds: vec![SeedPeer {
            peer: PeerId::from_bytes([4; 32]),
            address: "127.0.0.1:30303".to_owned(),
        }],
        lookup_timeout: Duration::from_millis(1),
        ..Default::default()
    };
    let peers = config
        .discover(|| async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Vec::new()
        })
        .await;
    assert_eq!(peers, config.seeds);
}

#[tokio::test]
async fn discovery_lookup_receives_seed_candidates_before_fallback() {
    let overlay = OverlayId::from_name(b"testnet");
    let seed = SeedPeer {
        peer: PeerId::from_bytes([4; 32]),
        address: "127.0.0.1:30303".to_owned(),
    };
    let config = DiscoveryConfig {
        overlay,
        seeds: vec![seed.clone()],
        ..Default::default()
    };
    let peers = config
        .discover_with(|seeds| async move {
            assert_eq!(seeds, vec![seed]);
            Vec::new()
        })
        .await;
    assert_eq!(peers, config.seeds);
}

#[tokio::test]
async fn reconnects_with_bounded_backoff_after_session_failure() {
    let manager = PeerManager::new(OverlayConfig::default()).unwrap();
    let peer = PeerId::from_bytes([6; 32]);
    let mut statuses = manager.subscribe_statuses();
    let reconnect: ReconnectFactory = Arc::new(move |candidate| {
        Box::pin(async move {
            assert_eq!(candidate, peer);
            Ok(Box::new(FlakySession { peer, fail: false }) as Box<dyn OverlaySession>)
        })
    });
    manager
        .add_session_with_reconnect(
            Box::new(FlakySession { peer, fail: true }),
            reconnect,
            2,
            Duration::from_millis(1),
        )
        .await;

    let mut reconnecting = false;
    tokio::time::timeout(Duration::from_secs(1), async {
        while let Ok(status) = statuses.recv().await {
            if matches!(status, PeerStatus::Reconnecting { .. }) {
                reconnecting = true;
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(reconnecting);
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(manager.peer_count().await, 1);
    manager.shutdown_wait().await;
    assert_eq!(manager.peer_count().await, 0);
}

#[test]
fn discovery_rejects_unparseable_addresses() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let mut record = DiscoveryRecord {
        overlay: OverlayId::from_name(b"test"),
        peer: PeerId::from_bytes([2; 32]),
        node_key: key.verifying_key().to_bytes(),
        address: "not-an-address".to_owned(),
        signature: [0; 64],
    };
    record.signature = key.sign(&record.signed_bytes()).to_bytes();
    assert!(!record.is_usable(record.overlay));
}

#[test]
fn typed_dht_selection_requires_signature_and_deduplicates_addresses() {
    let key = SigningKey::from_bytes(&[11; 32]);
    let node = tonutils_tl::tl::network::DhtNode {
        id: tonutils_tl::tl::network::PublicKey::Ed25519 {
            key: tonutils_tl::Int256(key.verifying_key().to_bytes()),
        },
        addr_list: tonutils_tl::tl::network::AddressList {
            addrs: vec![tonutils_tl::tl::network::Address::Udp {
                ip: 0x7f000001,
                port: 30303,
            }],
            version: 1,
            reinit_date: 1,
            priority: 0,
            expire_at: 0,
        },
        version: 1,
        signature: Vec::new(),
    };
    let unsigned = tonutils_tl::tl::network::DhtNodeBoxed {
        id: node.id.clone(),
        addr_list: node.addr_list.clone(),
        version: node.version,
        signature: Vec::new(),
    };
    let signature = key.sign(&tl_proto::serialize(unsigned));
    let mut signed = node;
    signed.signature = signature.to_bytes().to_vec();
    let peers = select_typed_dht_peers([signed.clone(), signed], 8, 2);
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer.as_bytes(), key.verifying_key().to_bytes());
    assert_eq!(peers[0].address, "127.0.0.1:30303");
}

#[test]
fn shard_public_overlay_id_is_deterministic() {
    let first = OverlayId::from_shard_public(-1, i64::MIN, [7; 32]);
    let second = OverlayId::from_shard_public(-1, i64::MIN, [7; 32]);
    assert_eq!(first, second);
    assert_ne!(first, OverlayId::from_shard_public(0, i64::MIN, [7; 32]));
}

#[test]
fn overlay_id_uses_boxed_public_overlay_key() {
    let zero_state_file_hash: [u8; 32] =
        hex::decode("5e994fcf4d425c0a6ce6a792594b7173205f740a39cd56f537defd28b48a0f6e")
            .unwrap()
            .try_into()
            .unwrap();
    assert_eq!(
        OverlayId::from_shard_public(0, i64::MIN, zero_state_file_hash).to_string(),
        "12b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649"
    );
}

/// Session whose `receive()` never completes, used to exercise the pool's idle
/// deadline.
struct PendingSession {
    peer: PeerId,
    /// Reports a fresh activity stamp on every poll, like a session that keeps
    /// consuming protocol traffic without completing `receive()`.
    active: bool,
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

    fn last_activity(&self) -> Option<std::time::Instant> {
        self.active.then(std::time::Instant::now)
    }
}

#[tokio::test]
async fn keeps_session_alive_while_protocol_activity_continues() {
    let manager = PeerManager::new(OverlayConfig {
        peer_idle_timeout: Duration::from_millis(20),
        ..Default::default()
    })
    .unwrap();
    let peer = PeerId::from_bytes([7; 32]);
    manager
        .add_session(Box::new(PendingSession { peer, active: true }))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(manager.peer_count().await, 1);
    manager.shutdown_wait().await;
    assert_eq!(manager.peer_count().await, 0);
}

#[tokio::test]
async fn drops_session_after_idle_deadline_without_activity() {
    let manager = PeerManager::new(OverlayConfig {
        peer_idle_timeout: Duration::from_millis(20),
        ..Default::default()
    })
    .unwrap();
    let peer = PeerId::from_bytes([8; 32]);
    manager
        .add_session(Box::new(PendingSession {
            peer,
            active: false,
        }))
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(manager.peer_count().await, 0);
    manager.shutdown_wait().await;
}
