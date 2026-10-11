//! Low-latency pending external-message scanner primitives.
//!
//! The fast path validates the outer BoC envelope, hashes the shared raw
//! bytes, performs bounded deduplication, and publishes an event. Full TL-B
//! decoding, storage, and LiteServer inclusion queries are intentionally left
//! to consumers or a future slow-path worker.

use futures::future::{BoxFuture, join_all};
use futures::stream::{self, Stream};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use tonutils_adnl::KeyPair;
use tonutils_network_config::{ConfigGlobal, extract_dht_addresses};
use tonutils_overlay::{
    DiscoveryConfig, OverlayConfig, OverlayId, OverlayPacket, OverlayPeerPool, OverlaySession,
    PeerId, PeerManager, PeerStatus, ReconnectFactory, RoutingMetadata, SeedDiscoveryLookup,
    SeedPeer,
};
use tonutils_overlay::{DiscoveryLookup, TypedDiscoveryLookup};
mod builder;
mod overlay_inbound;
mod protocol_stats;
mod quic_session;
mod udp_session;

pub use builder::{MempoolScannerBuilder, OverlaySessionFactory};
pub use protocol_stats::{ProtocolStats, protocol_stats};
pub use quic_session::{QuicOverlaySession, quic_overlay_factory, quic_overlay_lookup};
pub use udp_session::{
    AdnlUdpOverlaySession, DhtAddressPublisher, PUBLISH_INTERVAL, address_publisher,
    channel_factory, direct_factory, overlay_factory, udp_dht_lookup, udp_iterative_dht_lookup,
    udp_overlay_lookup, udp_peer_growth, udp_peer_growth_with_resolvers,
};

/// Delay between two peer-growth rounds after bootstrap.
///
/// Ten seconds matches pytoniq's `OverlayManager.get_more_peers` cadence: it
/// keeps re-announcing this node while it harvests members reachable only
/// through `overlay.getRandomPeers`.
const PEER_GROWTH_INTERVAL: Duration = Duration::from_secs(10);

/// Hash of a serialized external message.
pub type MessageHash = [u8; 32];

/// Scanner resource and validation limits.
#[derive(Clone, Debug)]
pub struct MempoolConfig {
    pub event_queue_capacity: usize,
    pub max_message_size: usize,
    pub dedup_shards: usize,
    pub require_boc_magic: bool,
    pub validate_message: bool,
    pub dedup_ttl: Duration,
    pub max_dedup_entries: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            event_queue_capacity: 1024,
            max_message_size: 1 << 20,
            dedup_shards: 32,
            require_boc_magic: true,
            validate_message: true,
            dedup_ttl: Duration::from_secs(300),
            max_dedup_entries: 100_000,
        }
    }
}

/// Events emitted by [`MempoolScanner`].
#[derive(Clone, Debug)]
pub enum MempoolEvent {
    ExternalMessage {
        hash: MessageHash,
        raw_boc: Arc<[u8]>,
        destination: Option<[u8; 32]>,
        routing: RoutingMetadata,
        timestamp: SystemTime,
    },
    Included {
        hash: MessageHash,
        block: Arc<[u8]>,
        transaction: Option<Arc<[u8]>>,
    },
    PeerStatus(PeerStatus),
}

impl MempoolEvent {
    /// Returns a lazy view for an accepted external message event.
    pub fn lazy_message(&self) -> Option<LazyExternalMessage> {
        match self {
            Self::ExternalMessage { hash, raw_boc, .. } => {
                Some(LazyExternalMessage::new(*hash, raw_boc.clone()))
            }
            _ => None,
        }
    }
}

/// A destination that can accept a raw external message for broadcast.
pub trait BroadcastPeer: Send + Sync {
    fn send_external(&self, raw_boc: Arc<[u8]>) -> BoxFuture<'static, Result<(), String>>;

    /// Stable identity used to avoid sending the same message twice through
    /// the same logical peer when several handles refer to it.
    fn peer_id(&self) -> Option<PeerId> {
        None
    }

    fn is_healthy(&self) -> bool {
        true
    }
}

/// Raw external message whose typed TL-B representation is decoded on demand.
#[derive(Clone, Debug)]
pub struct LazyExternalMessage {
    hash: MessageHash,
    raw_boc: Arc<[u8]>,
}

impl LazyExternalMessage {
    pub fn new(hash: MessageHash, raw_boc: Arc<[u8]>) -> Self {
        Self { hash, raw_boc }
    }

    pub fn hash(&self) -> MessageHash {
        self.hash
    }

    pub fn raw_boc(&self) -> Arc<[u8]> {
        self.raw_boc.clone()
    }

    pub fn decode(&self) -> Result<tonutils_tlb::Message, MempoolError> {
        let cell = tonutils_tvm::deserialize_boc(&self.raw_boc)
            .map_err(|error| MempoolError::Decode(error.to_string()))?;
        tonutils_tlb::TlbDeserialize::from_cell(cell)
            .map_err(|error| MempoolError::Decode(error.to_string()))
    }

    pub fn destination(&self) -> Result<Option<[u8; 32]>, MempoolError> {
        let message = self.decode()?;
        Ok(match message.info {
            tonutils_tlb::CommonMsgInfo::ExternalIn { dest, .. } => match dest {
                tonutils_tlb::MsgAddressInt::Std { address, .. } => Some(address.hash_part),
                tonutils_tlb::MsgAddressInt::Var { .. } => None,
            },
            _ => None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MempoolMetrics {
    pub accepted: u64,
    pub duplicates: u64,
    pub rejected: u64,
    pub broadcast_failures: u64,
    pub invalid_warnings: u64,
    pub overlay_packets: u64,
}

/// Outcome of the bootstrap discovery performed by
/// [`MempoolScannerBuilder::start`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryStats {
    /// Seed records resolved from configuration and bootstrap sources.
    pub seeds: u64,
    /// Overlay peers returned by the configured discovery lookup on top of
    /// the seeds. Zero means the lookup timed out, was empty, or was not
    /// configured, and the raw seeds were used as-is.
    pub discovered: u64,
    /// Total peer records handed to the session bootstrap.
    pub peers: u64,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MempoolError {
    #[error("mempool event queue capacity must be greater than zero")]
    InvalidQueueCapacity,
    #[error("mempool dedup shard count must be greater than zero")]
    InvalidShardCount,
    #[error("mempool dedup entry limit must be greater than zero")]
    InvalidDedupCapacity,
    #[error("external message is empty or smaller than its envelope")]
    InvalidEnvelope,
    #[error("external message is not a serialized BoC")]
    InvalidBoc,
    #[error("external BoC is not a valid external message")]
    InvalidMessage,
    #[error("external message exceeds configured size limit")]
    MessageTooLarge,
    #[error("mempool event queue is closed")]
    QueueClosed,
    #[error("external message failed lazy TL-B decoding: {0}")]
    Decode(String),
    #[error("mempool scanner has no validated bootstrap peers")]
    NoBootstrapPeers,
    #[error("invalid bootstrap peer address: {0}")]
    InvalidBootstrapAddress(String),
    #[error("global configuration download failed: {0}")]
    ConfigDownload(String),
    #[error("overlay manager initialization failed: {0}")]
    Overlay(String),
    #[error("overlay session connection failed: {0}")]
    Session(String),
}

/// Controls how callers configure the scanner's bounded event queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueuePolicy {
    /// Backpressure the producer when the consumer is slower than the network.
    Backpressure,
}

/// Optional local identity used by a future ADNL session factory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannerIdentity {
    pub public_key: [u8; 32],
}

/// A bounded, deduplicating scanner.
pub struct MempoolScanner {
    config: MempoolConfig,
    event_tx: mpsc::Sender<MempoolEvent>,
    event_rx: Arc<Mutex<mpsc::Receiver<MempoolEvent>>>,
    dedup: Arc<Vec<Mutex<HashMap<MessageHash, Instant>>>>,
    dedup_capacity: Mutex<()>,
    dedup_entries: AtomicU64,
    broadcast_peers: Arc<Mutex<Vec<Arc<dyn BroadcastPeer>>>>,
    accepted: AtomicU64,
    duplicates: AtomicU64,
    rejected: AtomicU64,
    broadcast_failures: AtomicU64,
    invalid_warnings: AtomicU64,
    overlay_packets: AtomicU64,
    discovery_seeds: AtomicU64,
    discovery_results: AtomicU64,
    discovery_peers: AtomicU64,
}

impl MempoolScanner {
    pub fn new(config: MempoolConfig) -> Result<Self, MempoolError> {
        if config.event_queue_capacity == 0 {
            return Err(MempoolError::InvalidQueueCapacity);
        }
        if config.dedup_shards == 0 {
            return Err(MempoolError::InvalidShardCount);
        }
        if config.max_dedup_entries == 0 {
            return Err(MempoolError::InvalidDedupCapacity);
        }
        let (event_tx, event_rx) = mpsc::channel(config.event_queue_capacity);
        let dedup = (0..config.dedup_shards)
            .map(|_| Mutex::new(HashMap::new()))
            .collect();
        Ok(Self {
            config,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            dedup: Arc::new(dedup),
            dedup_capacity: Mutex::new(()),
            dedup_entries: AtomicU64::new(0),
            broadcast_peers: Arc::new(Mutex::new(Vec::new())),
            accepted: AtomicU64::new(0),
            duplicates: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            broadcast_failures: AtomicU64::new(0),
            invalid_warnings: AtomicU64::new(0),
            overlay_packets: AtomicU64::new(0),
            discovery_seeds: AtomicU64::new(0),
            discovery_results: AtomicU64::new(0),
            discovery_peers: AtomicU64::new(0),
        })
    }

    /// Returns the scanner's Rust stream. The receiver is shared so ingest and
    /// outbound broadcast can continue while a consumer is polling the stream.
    pub fn events(&self) -> impl Stream<Item = MempoolEvent> + use<> {
        let receiver = self.event_rx.clone();
        stream::unfold(receiver, |receiver| async move {
            let event = receiver.lock().await.recv().await?;
            Some((event, receiver))
        })
    }

    pub async fn run_handler<H, F>(&self, mut handler: H)
    where
        H: FnMut(MempoolEvent) -> F,
        F: std::future::Future<Output = ()>,
    {
        let mut events = Box::pin(self.events());
        while let Some(event) = futures::StreamExt::next(&mut events).await {
            handler(event).await;
        }
    }

    pub async fn add_broadcast_peer(&self, peer: Arc<dyn BroadcastPeer>) {
        self.broadcast_peers.lock().await.push(peer);
    }

    pub fn metrics(&self) -> MempoolMetrics {
        MempoolMetrics {
            accepted: self.accepted.load(Ordering::Relaxed),
            duplicates: self.duplicates.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            broadcast_failures: self.broadcast_failures.load(Ordering::Relaxed),
            invalid_warnings: self.invalid_warnings.load(Ordering::Relaxed),
            overlay_packets: self.overlay_packets.load(Ordering::Relaxed),
        }
    }

    /// Returns the outcome of the bootstrap discovery performed by
    /// [`MempoolScannerBuilder::start`]: how many seeds were resolved, how
    /// many extra overlay peers the discovery lookup returned, and how many
    /// peer records were handed to the session bootstrap.
    ///
    /// `discovered == 0` means the configured lookup timed out, returned
    /// nothing, or was not configured, so the scanner fell back to the raw
    /// seeds.
    pub fn discovery_stats(&self) -> DiscoveryStats {
        DiscoveryStats {
            seeds: self.discovery_seeds.load(Ordering::Relaxed),
            discovered: self.discovery_results.load(Ordering::Relaxed),
            peers: self.discovery_peers.load(Ordering::Relaxed),
        }
    }

    fn record_discovery(&self, stats: DiscoveryStats) {
        self.discovery_seeds.store(stats.seeds, Ordering::Relaxed);
        self.discovery_results
            .store(stats.discovered, Ordering::Relaxed);
        self.discovery_peers.store(stats.peers, Ordering::Relaxed);
    }

    /// Returns process-wide overlay protocol counters.
    ///
    /// Unlike [`Self::metrics`], the counters are global: overlay sessions
    /// live in the peer pool, not in the scanner, so a snapshot explains what
    /// *all* sessions did — how many queries arrived, how many were wrapped in
    /// `overlay.query`, how many pings were answered, and whether peers listed
    /// this node among the verified members. See [`ProtocolStats`].
    pub fn protocol_stats(&self) -> ProtocolStats {
        protocol_stats()
    }

    /// Validates, deduplicates, publishes, and broadcasts one external BoC.
    pub async fn send_external(
        &self,
        raw_boc: impl Into<Arc<[u8]>>,
    ) -> Result<MessageHash, MempoolError> {
        let raw_boc = raw_boc.into();
        let hash = Sha256::digest(&raw_boc).into();
        let accepted = self
            .accept_fast(
                raw_boc.clone(),
                RoutingMetadata::new(OverlayId::from_name(b"local"), PeerId::from_bytes([0; 32])),
            )
            .await?;
        if accepted.is_some() {
            let peers = self.broadcast_peers.lock().await.clone();
            let mut peer_ids = HashSet::new();
            let peers = peers
                .into_iter()
                .filter(|peer| peer.peer_id().map(|id| peer_ids.insert(id)).unwrap_or(true))
                .filter(|peer| peer.is_healthy())
                .collect::<Vec<_>>();
            let results = join_all(
                peers
                    .into_iter()
                    .map(|peer| peer.send_external(raw_boc.clone())),
            )
            .await;
            self.broadcast_failures.fetch_add(
                results.iter().filter(|result| result.is_err()).count() as u64,
                Ordering::Relaxed,
            );
        }
        Ok(hash)
    }

    /// Accepts a packet received from an overlay peer.
    pub async fn ingest(
        &self,
        raw_boc: impl Into<Arc<[u8]>>,
        routing: RoutingMetadata,
    ) -> Result<Option<MessageHash>, MempoolError> {
        let raw_boc = raw_boc.into();
        let hash = self.accept_fast(raw_boc, routing).await?;
        Ok(hash)
    }

    /// Emits a later inclusion result without changing the initial `Seen`
    /// semantics of [`MempoolEvent::ExternalMessage`].
    pub async fn mark_included(
        &self,
        hash: MessageHash,
        block: impl Into<Arc<[u8]>>,
        transaction: Option<Arc<[u8]>>,
    ) -> Result<(), MempoolError> {
        self.event_tx
            .send(MempoolEvent::Included {
                hash,
                block: block.into(),
                transaction,
            })
            .await
            .map_err(|_| MempoolError::QueueClosed)
    }

    pub async fn peer_status(&self, status: PeerStatus) -> Result<(), MempoolError> {
        self.event_tx
            .send(MempoolEvent::PeerStatus(status))
            .await
            .map_err(|_| MempoolError::QueueClosed)
    }

    /// Starts the live receive path for a bounded overlay pool.
    #[must_use]
    pub fn spawn_overlay_receiver(
        self: Arc<Self>,
        pool: Arc<OverlayPeerPool>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(OverlayPacket { payload, routing }) = pool.next_packet().await {
                self.overlay_packets.fetch_add(1, Ordering::Relaxed);
                let pkt_size = payload.len();
                let pkt_prefix = hex::encode(&payload[..pkt_size.min(8)]);
                match self.ingest(payload, routing).await {
                    Ok(_) => {}
                    Err(MempoolError::QueueClosed) => break,
                    Err(error) => {
                        log::warn!(
                            "dropping invalid overlay packet: size={pkt_size} first_bytes={pkt_prefix} error={error}",
                        );
                        self.invalid_warnings.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
    }

    pub fn spawn_overlay_receiver_with_shutdown(
        self: Arc<Self>,
        pool: Arc<OverlayPeerPool>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    packet = pool.next_packet() => {
                        let Some(OverlayPacket { payload, routing }) = packet else { break; };
                        self.overlay_packets.fetch_add(1, Ordering::Relaxed);
                        let pkt_size = payload.len();
                        let pkt_prefix = hex::encode(&payload[..pkt_size.min(8)]);
                        match self.ingest(payload, routing).await {
                            Ok(_) => {}
                            Err(MempoolError::QueueClosed) => break,
                            Err(error) => {
                                log::warn!(
                                    "dropping invalid overlay packet: size={pkt_size} first_bytes={pkt_prefix} error={error}",
                                );
                                self.invalid_warnings.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        })
    }

    async fn accept_fast(
        &self,
        raw_boc: Arc<[u8]>,
        routing: RoutingMetadata,
    ) -> Result<Option<MessageHash>, MempoolError> {
        let destination = match validate_envelope(&raw_boc, &self.config) {
            Ok(destination) => destination,
            Err(error) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        let hash: MessageHash = Sha256::digest(&raw_boc).into();
        let shard = usize::from(hash[0]) % self.dedup.len();
        let now = Instant::now();
        {
            let _capacity_guard = self.dedup_capacity.lock().await;
            let mut dedup = self.dedup[shard].lock().await;
            let expired = dedup
                .iter()
                .filter(|(_, seen)| now.duration_since(**seen) >= self.config.dedup_ttl)
                .count();
            dedup.retain(|_, seen| now.duration_since(*seen) < self.config.dedup_ttl);
            if expired != 0 {
                self.dedup_entries
                    .fetch_sub(expired as u64, Ordering::Relaxed);
            }
            if dedup.contains_key(&hash) {
                self.duplicates.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            if self.dedup_entries.load(Ordering::Relaxed) >= self.config.max_dedup_entries as u64
                && dedup.is_empty()
            {
                drop(dedup);
                self.evict_oldest().await;
                dedup = self.dedup[shard].lock().await;
                if dedup.contains_key(&hash) {
                    self.duplicates.fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
            }
            dedup.insert(hash, now);
            self.dedup_entries.fetch_add(1, Ordering::Relaxed);
        }
        self.accepted.fetch_add(1, Ordering::Relaxed);
        self.event_tx
            .send(MempoolEvent::ExternalMessage {
                hash,
                raw_boc,
                destination,
                routing,
                timestamp: SystemTime::now(),
            })
            .await
            .map_err(|_| MempoolError::QueueClosed)?;
        Ok(Some(hash))
    }

    async fn evict_oldest(&self) {
        let mut candidates: Vec<([u8; 32], usize, Instant)> = Vec::new();
        for (index, shard) in self.dedup.iter().enumerate() {
            let dedup = shard.lock().await;
            for (hash, seen) in dedup.iter() {
                candidates.push((*hash, index, *seen));
            }
        }
        let evict_count = (self.config.max_dedup_entries / 10).max(1);
        candidates.sort_by_key(|(_, _, seen)| *seen);
        let mut removed = 0u64;
        for (hash, index, _) in candidates.into_iter().take(evict_count) {
            if self.dedup[index].lock().await.remove(&hash).is_some() {
                removed += 1;
            }
        }
        if removed > 0 {
            self.dedup_entries.fetch_sub(removed, Ordering::Relaxed);
        }
    }
}

fn validate_envelope(
    raw_boc: &[u8],
    config: &MempoolConfig,
) -> Result<Option<[u8; 32]>, MempoolError> {
    if raw_boc.len() < 4 {
        return Err(MempoolError::InvalidEnvelope);
    }
    if raw_boc.len() > config.max_message_size {
        return Err(MempoolError::MessageTooLarge);
    }
    if config.require_boc_magic && raw_boc[..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err(MempoolError::InvalidBoc);
    }
    if config.validate_message {
        let cell = tonutils_tvm::deserialize_boc(raw_boc).map_err(|_| MempoolError::InvalidBoc)?;
        let message: tonutils_tlb::Message = tonutils_tlb::TlbDeserialize::from_cell(cell)
            .map_err(|_| MempoolError::InvalidMessage)?;
        let destination = match message.info {
            tonutils_tlb::CommonMsgInfo::ExternalIn { dest, .. } => match dest {
                tonutils_tlb::MsgAddressInt::Std { address, .. } => Some(address.hash_part),
                tonutils_tlb::MsgAddressInt::Var { .. } => None,
            },
            _ => return Err(MempoolError::InvalidMessage),
        };
        return Ok(destination);
    }
    Ok(None)
}

#[cfg(test)]
#[path = "mempool_tests.rs"]
mod tests;
