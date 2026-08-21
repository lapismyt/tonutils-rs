//! Building blocks for TON overlay networks.
//!
//! This crate deliberately separates protocol-neutral peer management from
//! discovery and wire schemas. It provides bounded fan-out, peer scoring, and
//! routing metadata that can be driven by an ADNL session implementation.
//! DHT and overlay packet schemas are not guessed here; they are tracked in
//! `docs/reference/network/overlay.md` until upstream fixtures are available.

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(feature = "runtime")]
use futures::future::BoxFuture;
#[cfg(feature = "runtime")]
use tokio::sync::{RwLock, broadcast, mpsc, watch};

/// A 256-bit overlay identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OverlayId([u8; 32]);

impl OverlayId {
    /// Derives an overlay ID from its canonical name.
    pub fn from_name(name: &[u8]) -> Self {
        Self(Sha256::digest(name).into())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for OverlayId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Stable identifier for an overlay peer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Metadata attached by the receive path without decoding the payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingMetadata {
    pub overlay: OverlayId,
    pub peer: PeerId,
    pub received_at: SystemTime,
    pub hop: u8,
}

impl RoutingMetadata {
    pub fn new(overlay: OverlayId, peer: PeerId) -> Self {
        Self {
            overlay,
            peer,
            received_at: SystemTime::now(),
            hop: 0,
        }
    }

    pub fn received_unix_millis(&self) -> u128 {
        self.received_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    }
}

/// State changes emitted by the peer pool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerStatus {
    Connected { peer: PeerId },
    Disconnected { peer: PeerId },
    Failed { peer: PeerId },
    Reconnecting { peer: PeerId, attempt: u32 },
}

/// A transport-neutral authenticated overlay session.
#[cfg(feature = "runtime")]
pub trait OverlaySession: Send {
    fn peer_id(&self) -> PeerId;
    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>>;
    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>>;
}

#[cfg(feature = "runtime")]
type SharedSession = Arc<tokio::sync::Mutex<Box<dyn OverlaySession>>>;

/// A peer advertised by an explicit seed or a DHT lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedPeer {
    pub peer: PeerId,
    pub address: String,
}

/// Canonical signed discovery record.  The verifier is intentionally kept
/// local to this crate so callers can reject records before opening sessions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryRecord {
    pub overlay: OverlayId,
    pub peer: PeerId,
    pub node_key: [u8; 32],
    pub address: String,
    pub signature: [u8; 64],
}

impl DiscoveryRecord {
    pub fn signed_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + 32 + 32 + 2 + self.address.len());
        bytes.extend_from_slice(&self.overlay.as_bytes());
        bytes.extend_from_slice(&self.peer.as_bytes());
        bytes.extend_from_slice(&self.node_key);
        bytes.extend_from_slice(&(self.address.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.address.as_bytes());
        bytes
    }

    pub fn verify(&self) -> bool {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let Ok(key) = VerifyingKey::from_bytes(&self.node_key) else {
            return false;
        };
        key.verify(
            &self.signed_bytes(),
            &Signature::from_bytes(&self.signature),
        )
        .is_ok()
    }

    pub fn is_usable(&self, overlay: OverlayId) -> bool {
        self.overlay == overlay
            && self.address.len() <= 256
            && self.address.parse::<std::net::SocketAddr>().is_ok()
            && self.verify()
    }
}

/// DHT-first discovery policy with deterministic seed fallback.
#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    pub overlay: OverlayId,
    pub seeds: Vec<SeedPeer>,
    pub lookup_timeout: Duration,
    pub max_records: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            overlay: OverlayId::from_bytes([0; 32]),
            seeds: Vec::new(),
            lookup_timeout: Duration::from_secs(5),
            max_records: 64,
        }
    }
}

pub fn select_discovery_peers(
    config: &DiscoveryConfig,
    records: impl IntoIterator<Item = DiscoveryRecord>,
) -> Vec<SeedPeer> {
    let mut peers = HashSet::new();
    let mut selected = Vec::new();
    for record in records {
        if selected.len() >= config.max_records
            || record.overlay != config.overlay
            || !record.is_usable(config.overlay)
        {
            continue;
        }
        if peers.insert(record.peer) {
            selected.push(SeedPeer {
                peer: record.peer,
                address: record.address,
            });
        }
    }
    if selected.is_empty() {
        selected.extend(config.seeds.iter().cloned());
    }
    selected
}

#[cfg(feature = "runtime")]
impl DiscoveryConfig {
    /// Runs a DHT lookup first and deterministically falls back to configured
    /// seeds when the lookup times out or produces no usable records.
    pub async fn discover<F, Fut>(&self, lookup: F) -> Vec<SeedPeer>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Vec<DiscoveryRecord>>,
    {
        let records = tokio::time::timeout(self.lookup_timeout, lookup())
            .await
            .unwrap_or_default();
        select_discovery_peers(self, records)
    }
}

/// A bounded packet accepted by the overlay receive path.
#[derive(Clone, Debug)]
pub struct OverlayPacket {
    pub payload: Arc<[u8]>,
    pub routing: RoutingMetadata,
}

/// Configuration for a bounded overlay peer pool.
#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub queue_capacity: usize,
    pub max_packet_size: usize,
    pub peer_idle_timeout: Duration,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            max_packet_size: 1 << 20,
            peer_idle_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Error)]
pub enum OverlayError {
    #[error("overlay queue capacity must be greater than zero")]
    InvalidCapacity,
    #[error("overlay packet exceeds configured size limit")]
    PacketTooLarge,
    #[error("overlay peer is not registered")]
    UnknownPeer,
    #[error("overlay receive queue is closed")]
    QueueClosed,
}

/// Bounded peer pool used by scanners and broadcasters.
#[cfg(feature = "runtime")]
pub struct OverlayPeerPool {
    config: OverlayConfig,
    packets: mpsc::Sender<OverlayPacket>,
    packet_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<OverlayPacket>>>,
    statuses: broadcast::Sender<PeerStatus>,
    peers: Arc<RwLock<HashSet<PeerId>>>,
}

/// Persistent peer lifecycle manager with parallel receive loops and bounded
/// fan-out.  Session implementations own transport-specific handshakes.
#[cfg(feature = "runtime")]
pub struct PeerManager {
    pool: Arc<OverlayPeerPool>,
    sessions: Arc<RwLock<HashMap<PeerId, SharedSession>>>,
    scores: Arc<RwLock<HashMap<PeerId, i32>>>,
    shutdown: watch::Sender<bool>,
    overlay: OverlayId,
}

#[cfg(feature = "runtime")]
impl PeerManager {
    pub fn new(config: OverlayConfig) -> Result<Self, OverlayError> {
        Self::with_overlay(config, OverlayId::from_bytes([0; 32]))
    }

    pub fn with_overlay(config: OverlayConfig, overlay: OverlayId) -> Result<Self, OverlayError> {
        let (shutdown, _) = watch::channel(false);
        Ok(Self {
            pool: Arc::new(OverlayPeerPool::new(config)?),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            scores: Arc::new(RwLock::new(HashMap::new())),
            shutdown,
            overlay,
        })
    }

    pub fn pool(&self) -> Arc<OverlayPeerPool> {
        self.pool.clone()
    }

    /// Subscribes to connection and failure notifications from the manager.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.pool.subscribe_statuses()
    }

    pub async fn add_session(&self, session: Box<dyn OverlaySession>) {
        let peer = session.peer_id();
        let session = Arc::new(tokio::sync::Mutex::new(session));
        self.sessions.write().await.insert(peer, session.clone());
        self.scores.write().await.entry(peer).or_insert(0);
        self.pool.register_peer(peer).await;
        let pool = self.pool.clone();
        let overlay = self.overlay;
        let mut shutdown = self.shutdown.subscribe();
        let sessions = self.sessions.clone();
        let scores = self.scores.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    packet = async { session.lock().await.receive().await } => {
                        match packet {
                            Ok(payload) => {
                                let packet = OverlayPacket { payload, routing: RoutingMetadata::new(overlay, peer) };
                                if pool.ingest(packet).await.is_err() { break; }
                                scores.write().await.entry(peer).and_modify(|score| *score += 1);
                            }
                            Err(_) => {
                                let _ = pool.report_status(PeerStatus::Failed { peer }).await;
                                break;
                            }
                        }
                    }
                }
            }
            sessions.write().await.remove(&peer);
            pool.unregister_peer(peer).await;
            scores
                .write()
                .await
                .entry(peer)
                .and_modify(|score| *score -= 1);
        });
    }

    pub async fn broadcast(&self, payload: Arc<[u8]>) -> usize {
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let results = futures::future::join_all(sessions.into_iter().map(|session| {
            let payload = payload.clone();
            async move { session.lock().await.send(payload).await }
        }))
        .await;
        results.into_iter().flatten().count()
    }

    pub async fn score(&self, peer: PeerId) -> i32 {
        self.scores
            .read()
            .await
            .get(&peer)
            .copied()
            .unwrap_or_default()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

#[cfg(feature = "runtime")]
impl OverlayPeerPool {
    pub fn new(config: OverlayConfig) -> Result<Self, OverlayError> {
        if config.queue_capacity == 0 {
            return Err(OverlayError::InvalidCapacity);
        }
        let (packets, packet_rx) = mpsc::channel(config.queue_capacity);
        let (statuses, _) = broadcast::channel(config.queue_capacity);
        Ok(Self {
            config,
            packets,
            packet_rx: Arc::new(tokio::sync::Mutex::new(packet_rx)),
            statuses,
            peers: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    pub async fn register_peer(&self, peer: PeerId) {
        self.peers.write().await.insert(peer);
        let _ = self.statuses.send(PeerStatus::Connected { peer });
    }

    pub async fn unregister_peer(&self, peer: PeerId) {
        self.peers.write().await.remove(&peer);
        let _ = self.statuses.send(PeerStatus::Disconnected { peer });
    }

    pub async fn ingest(&self, packet: OverlayPacket) -> Result<(), OverlayError> {
        if packet.payload.len() > self.config.max_packet_size {
            return Err(OverlayError::PacketTooLarge);
        }
        if !self.peers.read().await.contains(&packet.routing.peer) {
            return Err(OverlayError::UnknownPeer);
        }
        self.packets
            .send(packet)
            .await
            .map_err(|_| OverlayError::QueueClosed)
    }

    pub async fn next_packet(&self) -> Option<OverlayPacket> {
        self.packet_rx.lock().await.recv().await
    }

    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.statuses.subscribe()
    }

    pub fn packet_sender(&self) -> mpsc::Sender<OverlayPacket> {
        self.packets.clone()
    }

    pub(crate) async fn report_status(&self, status: PeerStatus) -> Result<(), OverlayError> {
        self.statuses
            .send(status)
            .map(|_| ())
            .map_err(|_| OverlayError::QueueClosed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[tokio::test]
    async fn rejects_unknown_and_oversized_packets() {
        let pool = OverlayPeerPool::new(OverlayConfig {
            max_packet_size: 2,
            ..Default::default()
        })
        .unwrap();
        let packet = OverlayPacket {
            payload: Arc::from([1, 2, 3].as_slice()),
            routing: RoutingMetadata::new(
                OverlayId::from_name(b"test"),
                PeerId::from_bytes([1; 32]),
            ),
        };
        assert!(matches!(
            pool.ingest(packet).await,
            Err(OverlayError::PacketTooLarge)
        ));
        pool.register_peer(PeerId::from_bytes([1; 32])).await;
        let packet = OverlayPacket {
            payload: Arc::from([1].as_slice()),
            routing: RoutingMetadata::new(
                OverlayId::from_name(b"test"),
                PeerId::from_bytes([1; 32]),
            ),
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
}
