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
mod udp_session;

pub use udp_session::{
    AdnlUdpOverlaySession, channel_factory, direct_factory, overlay_factory, udp_dht_lookup,
    udp_iterative_dht_lookup, udp_overlay_lookup,
};

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

/// Builder for a live scanner startup.
pub type OverlaySessionFactory = Arc<
    dyn Fn(SeedPeer) -> BoxFuture<'static, Result<Box<dyn OverlaySession>, String>> + Send + Sync,
>;

#[derive(Clone)]
pub struct MempoolScannerBuilder {
    identity: Option<ScannerIdentity>,
    testnet: bool,
    explicit_seeds: Vec<SeedPeer>,
    global_config: Option<ConfigGlobal>,
    global_config_json: Option<String>,
    config: MempoolConfig,
    overlay: OverlayConfig,
    overlay_id: OverlayId,
    bootstrap_timeout: Duration,
    discovery_timeout: Duration,
    queue_policy: QueuePolicy,
    download_config: bool,
    session_factory: Option<OverlaySessionFactory>,
    discovery_lookup: Option<DiscoveryLookup>,
    typed_discovery_lookup: Option<TypedDiscoveryLookup>,
    seed_discovery_lookup: Option<SeedDiscoveryLookup>,
    dht_overlay_key: Option<[u8; 32]>,
    reconnect_attempts: u32,
    reconnect_backoff: Duration,
}

impl fmt::Debug for MempoolScannerBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MempoolScannerBuilder")
            .field("identity", &self.identity)
            .field("testnet", &self.testnet)
            .field("explicit_seeds", &self.explicit_seeds)
            .field("global_config", &self.global_config.is_some())
            .field("global_config_json", &self.global_config_json.is_some())
            .field("config", &self.config)
            .field("overlay", &self.overlay)
            .field("overlay_id", &self.overlay_id)
            .field("bootstrap_timeout", &self.bootstrap_timeout)
            .field("discovery_timeout", &self.discovery_timeout)
            .field("queue_policy", &self.queue_policy)
            .field("download_config", &self.download_config)
            .field("session_factory", &self.session_factory.is_some())
            .field("discovery_lookup", &self.discovery_lookup.is_some())
            .field(
                "typed_discovery_lookup",
                &self.typed_discovery_lookup.is_some(),
            )
            .field(
                "seed_discovery_lookup",
                &self.seed_discovery_lookup.is_some(),
            )
            .field("dht_overlay_key", &self.dht_overlay_key.is_some())
            .field("reconnect_attempts", &self.reconnect_attempts)
            .field("reconnect_backoff", &self.reconnect_backoff)
            .finish()
    }
}

impl Default for MempoolScannerBuilder {
    fn default() -> Self {
        Self {
            identity: None,
            testnet: false,
            explicit_seeds: Vec::new(),
            global_config: None,
            global_config_json: None,
            config: MempoolConfig::default(),
            overlay: OverlayConfig::default(),
            overlay_id: OverlayId::from_name(b"mempool"),
            bootstrap_timeout: Duration::from_secs(10),
            discovery_timeout: Duration::from_secs(5),
            queue_policy: QueuePolicy::Backpressure,
            download_config: true,
            session_factory: None,
            discovery_lookup: None,
            typed_discovery_lookup: None,
            seed_discovery_lookup: None,
            dht_overlay_key: None,
            reconnect_attempts: 5,
            reconnect_backoff: Duration::from_secs(1),
        }
    }
}

impl MempoolScannerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn identity(mut self, identity: ScannerIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    pub fn testnet(mut self, testnet: bool) -> Self {
        self.testnet = testnet;
        self
    }

    pub fn seed(mut self, seed: SeedPeer) -> Self {
        self.explicit_seeds.push(seed);
        self
    }

    pub fn seeds(mut self, seeds: impl IntoIterator<Item = SeedPeer>) -> Self {
        self.explicit_seeds.extend(seeds);
        self
    }

    pub fn global_config(mut self, config: ConfigGlobal) -> Self {
        self.global_config = Some(config);
        self
    }

    pub fn global_config_json(mut self, json: impl Into<String>) -> Self {
        self.global_config_json = Some(json.into());
        self
    }

    pub fn config(mut self, config: MempoolConfig) -> Self {
        self.config = config;
        self
    }

    pub fn overlay_config(mut self, config: OverlayConfig) -> Self {
        self.overlay = config;
        self
    }

    pub fn overlay_id(mut self, overlay_id: OverlayId) -> Self {
        self.overlay_id = overlay_id;
        self
    }

    pub fn bootstrap_timeout(mut self, timeout: Duration) -> Self {
        self.bootstrap_timeout = timeout;
        self
    }

    pub fn discovery_timeout(mut self, timeout: Duration) -> Self {
        self.discovery_timeout = timeout;
        self
    }

    pub fn queue_policy(mut self, policy: QueuePolicy) -> Self {
        self.queue_policy = policy;
        self
    }

    pub fn download_config(mut self, enabled: bool) -> Self {
        self.download_config = enabled;
        self
    }

    /// Installs the transport-specific connector used for discovered peers.
    /// The factory owns ADNL handshakes and returns an authenticated overlay
    /// session, keeping protocol wire details out of the scanner.
    pub fn session_factory(mut self, factory: OverlaySessionFactory) -> Self {
        self.session_factory = Some(factory);
        self
    }

    /// Installs the DHT lookup used before explicit seed fallback.
    pub fn discovery_lookup(mut self, lookup: DiscoveryLookup) -> Self {
        self.discovery_lookup = Some(lookup);
        self
    }

    /// Configures the native UDP DHT/session path in one step.
    ///
    /// A `Some` channel timeout enables ADNL create/confirm before application
    /// traffic; `None` keeps direct authenticated packets for peers that do
    /// not advertise channel support.
    pub fn native_udp(
        self,
        local_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        channel_timeout: Option<Duration>,
    ) -> Self {
        let discovery_timeout = self.discovery_timeout;
        let overlay = self.overlay_id;
        let session_factory = overlay_factory(local_addr, local_keypair, overlay, channel_timeout);
        let builder = self.session_factory(session_factory);
        match builder.dht_overlay_key {
            Some(overlay_key) => builder.seed_discovery_lookup(udp_overlay_lookup(
                local_addr,
                local_keypair,
                overlay_key,
                16,
                discovery_timeout,
            )),
            None => builder,
        }
    }

    pub fn native_udp_for_shard_public(
        self,
        local_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        workchain: i32,
        shard: i64,
        zero_state_file_hash: [u8; 32],
        channel_timeout: Option<Duration>,
    ) -> Self {
        let overlay_key: [u8; 32] = Sha256::digest(tl_proto::serialize(
            tonutils_tl::tl::network::TonNodeShardPublicOverlayId {
                workchain,
                shard,
                zero_state_file_hash: tonutils_tl::Int256(zero_state_file_hash),
            },
        ))
        .into();
        self.dht_overlay_key(overlay_key)
            .native_udp(local_addr, local_keypair, channel_timeout)
    }

    /// Configures native UDP sessions for explicit seeds only.
    pub fn native_udp_seeds_only(
        mut self,
        local_addr: std::net::SocketAddr,
        local_keypair: KeyPair,
        channel_timeout: Option<Duration>,
    ) -> Self {
        self.download_config = false;
        self.global_config = None;
        self.global_config_json = None;
        self.discovery_lookup = None;
        self.typed_discovery_lookup = None;
        self.seed_discovery_lookup = None;
        let session_factory =
            overlay_factory(local_addr, local_keypair, self.overlay_id, channel_timeout);
        self.session_factory(session_factory)
    }

    pub fn typed_discovery_lookup(mut self, lookup: TypedDiscoveryLookup) -> Self {
        self.typed_discovery_lookup = Some(lookup);
        self
    }

    pub fn seed_discovery_lookup(mut self, lookup: SeedDiscoveryLookup) -> Self {
        self.seed_discovery_lookup = Some(lookup);
        self
    }

    /// Configures the full `pub.overlay` name used for DHT overlay-node lookup.
    /// The short [`OverlayId`] alone is not sufficient to reconstruct it.
    pub fn dht_overlay_key(mut self, overlay_key: [u8; 32]) -> Self {
        self.dht_overlay_key = Some(overlay_key);
        self
    }

    pub fn reconnect_attempts(mut self, attempts: u32) -> Self {
        self.reconnect_attempts = attempts;
        self
    }

    pub fn reconnect_backoff(mut self, backoff: Duration) -> Self {
        self.reconnect_backoff = backoff;
        self
    }

    /// Resolves bootstrap sources, initializes the bounded overlay manager, and
    /// starts the scanner's overlay receive adapter.
    pub async fn start(
        self,
    ) -> Result<
        (
            Arc<MempoolScanner>,
            PeerManager,
            impl Stream<Item = MempoolEvent>,
        ),
        MempoolError,
    > {
        let _identity = self.identity;
        let _ = self.queue_policy;
        let seeds = self.resolve_bootstrap().await?;
        let discovery = DiscoveryConfig {
            overlay: self.overlay_id,
            seeds: seeds.clone(),
            lookup_timeout: self.discovery_timeout,
            max_records: 64,
        };
        let peers = if let Some(lookup) = self.seed_discovery_lookup.clone() {
            tokio::time::timeout(self.discovery_timeout, lookup(seeds.clone()))
                .await
                .ok()
                .filter(|peers| !peers.is_empty())
                .unwrap_or(seeds)
        } else if let Some(lookup) = self.typed_discovery_lookup.clone() {
            discovery.discover_typed(move |seeds| lookup(seeds)).await
        } else if let Some(lookup) = self.discovery_lookup.clone() {
            discovery.discover_with(move |seeds| lookup(seeds)).await
        } else {
            discovery.discover(|| async { Vec::new() }).await
        };
        if peers.is_empty() {
            return Err(MempoolError::NoBootstrapPeers);
        }
        let manager = PeerManager::with_overlay(self.overlay, self.overlay_id)
            .map_err(|error| MempoolError::Overlay(error.to_string()))?;
        let scanner = Arc::new(MempoolScanner::new(self.config)?);
        let stream = scanner.events();
        let _receiver_task = scanner
            .clone()
            .spawn_overlay_receiver_with_shutdown(manager.pool(), manager.subscribe_shutdown());
        let mut statuses = manager.subscribe_statuses();
        let status_scanner = scanner.clone();
        let mut shutdown = manager.subscribe_shutdown();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    status = statuses.recv() => {
                        let Ok(status) = status else { break; };
                        if status_scanner.peer_status(status).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
        if let Some(factory) = self.session_factory {
            let reconnect_factory: ReconnectFactory = {
                let factory = factory.clone();
                let seeds = peers.clone();
                Arc::new(move |peer| {
                    let factory = factory.clone();
                    let seed = seeds.iter().find(|seed| seed.peer == peer).cloned();
                    Box::pin(async move {
                        let seed = seed.ok_or_else(|| "peer is no longer configured".to_owned())?;
                        factory(seed).await
                    })
                })
            };
            let results = join_all(peers.into_iter().map(|peer| {
                let factory = factory.clone();
                async move { factory(peer).await }
            }))
            .await;
            let mut connected = 0;
            for result in results {
                match result {
                    Ok(session) => {
                        manager
                            .add_session_with_reconnect(
                                session,
                                reconnect_factory.clone(),
                                self.reconnect_attempts,
                                self.reconnect_backoff,
                            )
                            .await;
                        connected += 1;
                    }
                    Err(error) => log::debug!("bootstrap session failed: {error}"),
                }
            }
            if connected == 0 {
                manager.shutdown();
                return Err(MempoolError::Session(
                    "all validated bootstrap sessions failed".into(),
                ));
            }
        }
        Ok((scanner, manager, stream))
    }

    async fn resolve_bootstrap(&self) -> Result<Vec<SeedPeer>, MempoolError> {
        let mut peers = self.explicit_seeds.clone();
        if let Some(config) = &self.global_config {
            peers.extend(
                config
                    .bootstrap_addresses()
                    .into_iter()
                    .map(|item| SeedPeer {
                        peer: PeerId::from_bytes(item.public_key.unwrap_or([0; 32])),
                        address: item.address.to_string(),
                    }),
            );
        }
        if let Some(json) = &self.global_config_json {
            peers.extend(parse_seed_json(json)?);
        }
        if self.download_config {
            let url = if self.testnet {
                "https://ton.org/testnet-global.config.json"
            } else {
                "https://ton.org/global.config.json"
            };
            match tokio::time::timeout(self.bootstrap_timeout, download_config(url)).await {
                Ok(Ok(json)) => peers.extend(parse_seed_json(&json)?),
                Ok(Err(error)) if peers.is_empty() => return Err(error),
                Err(_) if peers.is_empty() => {
                    return Err(MempoolError::ConfigDownload(
                        "bootstrap download timed out".into(),
                    ));
                }
                _ => {}
            }
        }
        let mut unique = HashMap::<(PeerId, String), SeedPeer>::new();
        for peer in peers {
            if !peer.is_valid() {
                return Err(MempoolError::InvalidBootstrapAddress(peer.address));
            }
            unique
                .entry((peer.peer, peer.address.clone()))
                .or_insert(peer);
        }
        let mut peers = unique.into_values().collect::<Vec<_>>();
        peers.sort_by(|left, right| {
            left.peer
                .as_bytes()
                .cmp(&right.peer.as_bytes())
                .then_with(|| left.address.cmp(&right.address))
        });
        Ok(peers)
    }
}

async fn download_config(url: &str) -> Result<String, MempoolError> {
    reqwest::get(url)
        .await
        .map_err(|error| MempoolError::ConfigDownload(error.to_string()))?
        .error_for_status()
        .map_err(|error| MempoolError::ConfigDownload(error.to_string()))?
        .text()
        .await
        .map_err(|error| MempoolError::ConfigDownload(error.to_string()))
}

fn parse_seed_json(json: &str) -> Result<Vec<SeedPeer>, MempoolError> {
    let addresses = extract_dht_addresses(json)
        .map_err(|error| MempoolError::ConfigDownload(error.to_string()))?;
    Ok(addresses
        .into_iter()
        .map(|item| SeedPeer {
            peer: PeerId::from_bytes(item.public_key.unwrap_or([0; 32])),
            address: item.address.to_string(),
        })
        .collect())
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
    last_invalid_warning: Mutex<Option<Instant>>,
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
            last_invalid_warning: Mutex::new(None),
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
        }
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
                if let Err(error) = self.ingest(payload, routing).await {
                    let now = Instant::now();
                    let mut last_warning = self.last_invalid_warning.lock().await;
                    if last_warning
                        .map(|last| now.duration_since(last) >= Duration::from_secs(1))
                        .unwrap_or(true)
                    {
                        log::warn!("dropping invalid overlay external message: {error}");
                        self.invalid_warnings.fetch_add(1, Ordering::Relaxed);
                        *last_warning = Some(now);
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
                        if let Err(error) = self.ingest(payload, routing).await {
                            let now = Instant::now();
                            let mut last_warning = self.last_invalid_warning.lock().await;
                            if last_warning
                                .map(|last| now.duration_since(last) >= Duration::from_secs(1))
                                .unwrap_or(true)
                            {
                                log::warn!("dropping invalid overlay external message: {error}");
                                self.invalid_warnings.fetch_add(1, Ordering::Relaxed);
                                *last_warning = Some(now);
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
            drop(dedup);
            if self.dedup_entries.load(Ordering::Relaxed) >= self.config.max_dedup_entries as u64 {
                self.evict_oldest().await;
            }
            self.dedup[shard].lock().await.insert(hash, now);
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
        let mut oldest = None;
        for (index, shard) in self.dedup.iter().enumerate() {
            let dedup = shard.lock().await;
            if let Some((hash, seen)) = dedup.iter().min_by_key(|(_, seen)| **seen)
                && oldest.is_none_or(|(_, _, current)| *seen < current)
            {
                oldest = Some((*hash, index, *seen));
            }
        }
        if let Some((hash, index, _)) = oldest
            && self.dedup[index].lock().await.remove(&hash).is_some()
        {
            self.dedup_entries.fetch_sub(1, Ordering::Relaxed);
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
