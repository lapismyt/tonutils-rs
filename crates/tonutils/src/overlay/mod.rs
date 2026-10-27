//! Building blocks for TON overlay networks.
//!
//! This crate deliberately separates protocol-neutral peer management from
//! discovery and wire schemas. It provides bounded fan-out, peer scoring, and
//! routing metadata that can be driven by an ADNL session implementation.
//! DHT and overlay packet schemas are not guessed here; they are tracked in
//! `docs/reference/network/overlay.md` until upstream fixtures are available.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    overlay::manager::{CancelRegistration, SharedSession},
    tl::{
        Int256,
        network::{Address, DhtNode, TlPublicKey, TonNodeShardPublicOverlayId},
    },
};

use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::{RwLock, broadcast, mpsc, watch};

/// A 256-bit overlay identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OverlayId([u8; 32]);

impl OverlayId {
    /// Derives an overlay ID from its canonical name.
    pub fn from_name(name: &[u8]) -> Self {
        Self(
            Sha256::digest(tl_proto::serialize(TlPublicKey::Overlay {
                name: name.to_vec(),
            }))
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
        let shard_id = Sha256::digest(tl_proto::serialize(TonNodeShardPublicOverlayId {
            workchain,
            shard,
            zero_state_file_hash: Int256(zero_state_file_hash),
        }));
        Self::from_name(&shard_id)
    }

    /// Mainnet basechain (workchain 0) overlay ID.
    ///
    /// Computed as `SHA256(TL(pub.overlay { name: SHA256(TL(tonNode.shardPublicOverlayId
    /// { 0, MIN_SHARD, MAINNET_ZERO_STATE_HASH })) }))` where
    /// `MAINNET_ZERO_STATE_HASH = 5e994fcf4d425c0a6ce6a792594b7173205f740a39cd56f537defd28b48a0f6e`.
    pub const MAINNET_BASECHAIN_OVERLAY_ID: Self = Self([
        0x12, 0xb8, 0xa8, 0x3f, 0x09, 0x8e, 0x15, 0xea, 0x47, 0xfe, 0x76, 0xd0, 0xb0, 0xdf, 0x09,
        0x86, 0xff, 0x6d, 0xda, 0x19, 0x80, 0x79, 0x6b, 0x08, 0x4b, 0x0d, 0x2a, 0x68, 0xb2, 0x55,
        0x86, 0x49,
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

/// A send handle onto an overlay session that bypasses the receive
/// loop's session lock.
///
/// A session receive loop holds its session's mutex while `receive()`
/// waits for - and consumes protocol traffic inside - the next inbound
/// payload, so a send through the shared session stalls on any member
/// that does not push this node custom broadcasts: the wait lasts until
/// the next broadcast packet from that member or until
/// [`OverlayConfig::peer_idle_timeout`] expires, and one such member
/// delays a whole fan-out.  Transports that are internally synchronized
/// (ADNL/UDP sessions are `Clone` over a shared `Arc`, QUIC sessions
/// share an `Arc` connection) expose a sender that never touches the
/// lock; sessions that cannot keep the default
/// [`OverlaySession::sender`] of `None` and fall back to the mutex.
pub trait OverlaySessionSender: Send + Sync {
    /// Sends a raw payload to the peer, wrapping it in
    /// `overlay.unicast` for overlay-aware sessions.
    fn send(&self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>>;

    /// Sends a serialized TL object to the peer; overlay-aware sessions
    /// frame it after the `overlay.message` header with no
    /// `overlay.unicast` wrapper.
    fn send_object(&self, object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>>;
}

/// A transport-neutral authenticated overlay session.
pub trait OverlaySession: Send {
    fn peer_id(&self) -> PeerId;
    fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>>;
    fn send(&mut self, payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>>;

    /// Sends a raw overlay object: `overlay.message{overlay_id}`
    /// followed by `object`, with no `overlay.unicast` wrapper.
    ///
    /// Overlay broadcasts (`overlay.broadcast`,
    /// `overlay.broadcastFec`) must ride as raw objects:
    /// upstream's `OverlayImpl::receive_message` dispatches on
    /// the object's constructor, so a broadcast wrapped in
    /// `overlay.unicast` would reach the overlay callback's
    /// plain-message handler instead of the broadcast machinery
    /// and never propagate to neighbours.  The default
    /// rejection keeps sessions that only implement the wrapped
    /// [`Self::send`] from silently mis-framing broadcasts.
    fn send_object(&mut self, _object: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Err("session does not support raw overlay objects".to_owned()) })
    }

    /// Last instant at which the session observed inbound transport traffic.
    ///
    /// [`OverlayConfig::peer_idle_timeout`] bounds a single `receive()` call,
    /// but sessions consume protocol messages (ADNL answers, overlay queries,
    /// channel negotiation) inside `receive()` without completing it.  A
    /// session that reports activity newer than the idle timeout is therefore
    /// kept instead of being dropped; the default `None` keeps the historical
    /// hard-deadline behaviour for sessions that cannot report activity.
    fn last_activity(&self) -> Option<std::time::Instant> {
        None
    }

    /// A lock-free send handle for this session, when the transport is
    /// internally synchronized.
    ///
    /// The broadcast paths prefer it over the shared session mutex, so
    /// sends never wait out a parked `receive()` (see
    /// [`OverlaySessionSender`]); the default `None` keeps sessions on
    /// the mutex path.
    fn sender(&self) -> Option<Arc<dyn OverlaySessionSender>> {
        None
    }
}

pub type ReconnectFactory = Arc<
    dyn Fn(PeerId) -> BoxFuture<'static, Result<Box<dyn OverlaySession>, String>> + Send + Sync,
>;

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

pub type DiscoveryLookup =
    std::sync::Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<DiscoveryRecord>> + Send + Sync>;

pub type TypedDiscoveryLookup =
    std::sync::Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<DhtNode>> + Send + Sync>;

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

/// Bounded wait on [`OverlayPeerPool::ingest`] from a session receive
/// loop.
///
/// The queue between the sessions and the consumer is bounded, and a
/// consumer stall must never freeze the loop that answers inline
/// overlay queries: peers keep a member in `overlay.getRandomPeers`
/// only while its pings are answered (pytoniq drops a peer after four
/// unanswered pings).  Once the wait expires the packet is dropped -
/// floods carry redundant copies anyway - and the loop goes straight
/// back to polling `receive()`.
const INGEST_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// Bounded peer pool used by scanners and broadcasters.
pub struct OverlayPeerPool {
    config: OverlayConfig,
    packets: mpsc::Sender<OverlayPacket>,
    packet_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<OverlayPacket>>>,
    statuses: broadcast::Sender<PeerStatus>,
    peers: Arc<RwLock<HashSet<PeerId>>>,
    /// Packets dropped by session receive loops after the bounded
    /// [`INGEST_WAIT_TIMEOUT`] elapsed with the queue still full.
    ingest_drops: AtomicU64,
}

/// Persistent peer lifecycle manager with parallel receive loops and bounded
/// fan-out.  Session implementations own transport-specific handshakes.
///
/// Cloning yields another handle onto the same sessions, scores and shutdown
/// channel, so background tasks such as periodic peer growth can register
/// sessions while the caller keeps the original handle.
#[derive(Clone)]
pub struct PeerManager {
    pool: Arc<OverlayPeerPool>,
    sessions: Arc<RwLock<HashMap<PeerId, SharedSession>>>,
    scores: Arc<RwLock<HashMap<PeerId, i32>>>,
    cancellations: Arc<RwLock<HashMap<PeerId, CancelRegistration>>>,
    cancel_seq: Arc<AtomicU64>,
    shutdown: watch::Sender<bool>,
    overlay: OverlayId,
    tasks: Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

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
            ingest_drops: AtomicU64::new(0),
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

    /// Counts one packet a session loop had to drop because the queue
    /// stayed full past [`INGEST_WAIT_TIMEOUT`], returning the new
    /// total.
    ///
    /// Called only from the session receive loops under that bounded
    /// wait; see [`Self::ingest_drops`] for the exposed counter.
    pub fn record_ingest_drop(&self) -> u64 {
        self.ingest_drops.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Packets dropped so far by session receive loops whose bounded
    /// wait on [`Self::ingest`] expired with the queue still full.
    ///
    /// A non-zero value means the pool consumer stalled long enough to
    /// fill the bounded queue; the drops themselves are the intended
    /// trade-off, since the loop keeps answering overlay queries
    /// instead of freezing.
    pub fn ingest_drops(&self) -> u64 {
        self.ingest_drops.load(Ordering::Relaxed)
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

mod manager;

#[cfg(test)]
mod tests;
