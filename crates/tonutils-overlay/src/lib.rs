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
use tonutils_tl::tl::network::{Address, DhtNode, PublicKey as TlPublicKey};

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
        Self(
            Sha256::digest(tl_proto::serialize(
                tonutils_tl::tl::network::PublicKey::Overlay {
                    name: name.to_vec(),
                },
            ))
            .into(),
        )
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn from_shard_public(workchain: i32, shard: i64, zero_state_file_hash: [u8; 32]) -> Self {
        let shard_id = Sha256::digest(tl_proto::serialize(
            tonutils_tl::tl::network::TonNodeShardPublicOverlayId {
                workchain,
                shard,
                zero_state_file_hash: tonutils_tl::Int256(zero_state_file_hash),
            },
        ));
        Self::from_name(&shard_id)
    }

    /// Mainnet basechain (workchain 0) overlay ID.
    ///
    /// Computed as `SHA256(TL(pub.overlay { name: SHA256(TL(tonNode.shardPublicOverlayId
    /// { 0, MIN_SHARD, MAINNET_ZERO_STATE_HASH })) }))` where
    /// `MAINNET_ZERO_STATE_HASH = 5e994fcf4d425c0a6ce6a792594b7173205f740a39cd56f537defd28b48a0f6e`.
    pub const MAINNET_BASECHAIN_OVERLAY_ID: Self = Self([
        0xe7, 0x5e, 0x65, 0x0c, 0x6d, 0xdf, 0xf5, 0x98, 0x91, 0xa3, 0xb3, 0x36, 0x2a, 0xa1, 0x2a,
        0x5c, 0x79, 0x11, 0xae, 0xef, 0xdc, 0x1b, 0x2b, 0xaa, 0x2f, 0x66, 0x01, 0xe4, 0x52, 0xcc,
        0xcc, 0x06,
    ]);
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
pub type ReconnectFactory = Arc<
    dyn Fn(PeerId) -> BoxFuture<'static, Result<Box<dyn OverlaySession>, String>> + Send + Sync,
>;

#[cfg(feature = "runtime")]
type SharedSession = Arc<tokio::sync::Mutex<Box<dyn OverlaySession>>>;

/// A peer advertised by an explicit seed or a DHT lookup.
///
/// `peer` contains the 32-byte Ed25519 public key used by native ADNL
/// connectors. It is kept in `PeerId` for compatibility with the transport-
/// neutral overlay manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeedPeer {
    pub peer: PeerId,
    pub address: String,
}

impl SeedPeer {
    #[must_use]
    pub fn from_public_key(public_key: [u8; 32], address: impl Into<String>) -> Self {
        Self {
            peer: PeerId::from_bytes(public_key),
            address: address.into(),
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.peer.as_bytes() != [0; 32]
            && self.address.len() <= 256
            && self
                .address
                .parse::<std::net::SocketAddr>()
                .is_ok_and(|address| address.port() != 0 && !address.ip().is_unspecified())
    }
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

#[cfg(feature = "runtime")]
pub type DiscoveryLookup =
    std::sync::Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<DiscoveryRecord>> + Send + Sync>;

#[cfg(feature = "runtime")]
pub type TypedDiscoveryLookup =
    std::sync::Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<DhtNode>> + Send + Sync>;

#[cfg(feature = "runtime")]
pub type SeedDiscoveryLookup =
    std::sync::Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<SeedPeer>> + Send + Sync>;

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

pub fn select_typed_dht_peers(
    records: impl IntoIterator<Item = DhtNode>,
    max_records: usize,
    now: i32,
) -> Vec<SeedPeer> {
    let mut peers = HashSet::new();
    let mut selected = Vec::new();
    let mut total = 0u32;
    let mut filtered_valid = 0u32;
    let mut filtered_key = 0u32;
    for record in records {
        total += 1;
        if selected.len() >= max_records {
            break;
        }
        if !record.is_valid(now) {
            filtered_valid += 1;
            log::trace!(
                "select_typed_dht_peers: skipping invalid DHT node version={} expire_at={}",
                record.version,
                record.addr_list.expire_at,
            );
            continue;
        }
        let TlPublicKey::Ed25519 { key } = record.id else {
            filtered_key += 1;
            continue;
        };
        let peer = PeerId::from_bytes(key.0);
        for address in record.addr_list.addrs {
            let Address::Udp { ip, port } = address else {
                continue;
            };
            let Ok(port) = u16::try_from(port) else {
                continue;
            };
            if port == 0 || ip == 0 {
                continue;
            }
            let address = format!("{}:{}", std::net::Ipv4Addr::from(ip.cast_unsigned()), port);
            if peers.insert((peer, address.clone())) {
                selected.push(SeedPeer { peer, address });
                if selected.len() >= max_records {
                    break;
                }
            }
        }
    }
    log::trace!(
        "select_typed_dht_peers: total={total} selected={} filtered_valid={filtered_valid} filtered_key={filtered_key}",
        selected.len(),
    );
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
        self.discover_with(|_| lookup()).await
    }

    pub async fn discover_with<F, Fut>(&self, lookup: F) -> Vec<SeedPeer>
    where
        F: FnOnce(Vec<SeedPeer>) -> Fut,
        Fut: std::future::Future<Output = Vec<DiscoveryRecord>>,
    {
        let records = tokio::time::timeout(self.lookup_timeout, lookup(self.seeds.clone()))
            .await
            .unwrap_or_default();
        select_discovery_peers(self, records)
    }

    pub async fn discover_typed<F, Fut>(&self, lookup: F) -> Vec<SeedPeer>
    where
        F: FnOnce(Vec<SeedPeer>) -> Fut,
        Fut: std::future::Future<Output = Vec<DhtNode>>,
    {
        let records = tokio::time::timeout(self.lookup_timeout, lookup(self.seeds.clone()))
            .await
            .unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .min(i32::MAX as u64) as i32;
        let selected = select_typed_dht_peers(records, self.max_records, now);
        if selected.is_empty() {
            self.seeds.clone()
        } else {
            selected
        }
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
    tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
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
            tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        })
    }

    pub fn pool(&self) -> Arc<OverlayPeerPool> {
        self.pool.clone()
    }

    /// Subscribes to connection and failure notifications from the manager.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.pool.subscribe_statuses()
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub async fn add_session(&self, session: Box<dyn OverlaySession>) {
        let peer = session.peer_id();
        if self.sessions.read().await.contains_key(&peer) {
            let _ = self
                .pool
                .report_status(PeerStatus::Reconnecting { peer, attempt: 0 });
            return;
        }
        let session = Arc::new(tokio::sync::Mutex::new(session));
        self.sessions.write().await.insert(peer, session.clone());
        self.scores.write().await.entry(peer).or_insert(0);
        self.pool.register_peer(peer).await;
        let pool = self.pool.clone();
        let overlay = self.overlay;
        let idle_timeout = pool.config.peer_idle_timeout;
        let mut shutdown = self.shutdown.subscribe();
        let sessions = self.sessions.clone();
        let scores = self.scores.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    packet = async {
                        tokio::time::timeout(idle_timeout, async {
                            session.lock().await.receive().await
                        }).await
                    } => {
                        match packet {
                            Ok(Ok(payload)) => {
                                let packet = OverlayPacket { payload, routing: RoutingMetadata::new(overlay, peer) };
                                if pool.ingest(packet).await.is_err() { break; }
                                scores.write().await.entry(peer).and_modify(|score| *score += 1);
                            }
                            Ok(Err(_)) | Err(_) => {
                                let _ = pool.report_status(PeerStatus::Failed { peer });
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
        self.tasks.lock().await.push(task);
    }

    pub async fn add_session_with_reconnect(
        &self,
        session: Box<dyn OverlaySession>,
        reconnect: ReconnectFactory,
        max_attempts: u32,
        base_backoff: Duration,
    ) {
        let peer = session.peer_id();
        if self.sessions.read().await.contains_key(&peer) {
            return;
        }
        let sessions = self.sessions.clone();
        let scores = self.scores.clone();
        let pool = self.pool.clone();
        let overlay = self.overlay;
        let idle_timeout = pool.config.peer_idle_timeout;
        let mut shutdown = self.shutdown.subscribe();
        let task = tokio::spawn(async move {
            let mut current = session;
            loop {
                let shared = Arc::new(tokio::sync::Mutex::new(current));
                sessions.write().await.insert(peer, shared.clone());
                pool.register_peer(peer).await;
                let _ = pool.report_status(PeerStatus::Connected { peer });
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        packet = async {
                            tokio::time::timeout(idle_timeout, async {
                                shared.lock().await.receive().await
                            }).await
                        } => {
                            match packet {
                                Ok(Ok(payload)) => {
                                    let packet = OverlayPacket { payload, routing: RoutingMetadata::new(overlay, peer) };
                                    if pool.ingest(packet).await.is_err() { return; }
                                    scores.write().await.entry(peer).and_modify(|score| *score += 1);
                                }
                                Ok(Err(_)) | Err(_) => {
                                    break;
                                }
                            }
                        }
                    }
                }
                sessions.write().await.remove(&peer);
                pool.unregister_peer(peer).await;
                if *shutdown.borrow() {
                    let _ = pool.report_status(PeerStatus::Disconnected { peer });
                    return;
                }
                if max_attempts == 0 {
                    let _ = pool.report_status(PeerStatus::Failed { peer });
                    return;
                }
                let _ = pool.report_status(PeerStatus::Failed { peer });
                let mut replacement = None;
                for attempt in 1..=max_attempts {
                    let _ = pool.report_status(PeerStatus::Reconnecting { peer, attempt });
                    let multiplier = 1u32
                        .checked_shl(attempt.saturating_sub(1))
                        .unwrap_or(u32::MAX);
                    let delay = base_backoff
                        .saturating_mul(multiplier)
                        .min(Duration::from_secs(30));
                    tokio::select! {
                        _ = shutdown.changed() => return,
                        () = tokio::time::sleep(delay) => {}
                    }
                    match reconnect(peer).await {
                        Ok(session) => {
                            replacement = Some(session);
                            break;
                        }
                        Err(error) => log::debug!("reconnect for {peer:?} failed: {error}"),
                    }
                }
                let Some(session) = replacement else {
                    return;
                };
                current = session;
            }
        });
        self.tasks.lock().await.push(task);
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

    pub async fn peer_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    pub async fn shutdown_wait(&self) {
        self.shutdown();
        let tasks = std::mem::take(&mut *self.tasks.lock().await);
        for task in tasks {
            let _ = task.await;
        }
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

    pub(crate) fn report_status(&self, status: PeerStatus) -> Result<(), OverlayError> {
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
        let unsigned = tonutils_tl::tl::network::DhtNode {
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
}
