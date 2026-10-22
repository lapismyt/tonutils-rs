//! `MempoolScannerBuilder`: bootstrap resolution and startup wiring.
//!
//! Split out of `lib.rs` to keep that file near the repository's 1000 line
//! limit; `lib.rs` re-exports [`MempoolScannerBuilder`].

use super::*;
use crate::bootstrap::{
    BootstrapDial, await_registered, download_config, merge_peer_sets, parse_seed_json,
    warm_store_fallback,
};
use crate::discovery_task::{PeerDiscoveryTask, discovery_runner};
use crate::peer_growth::PeerGrowthTask;
use crate::peer_store::{PeerStore, PeerStoreWriter};

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
    peer_growth_lookup: Option<SeedDiscoveryLookup>,
    /// DHT resolver seeds shared with the peer-growth lookup.
    ///
    /// `start` fills this with the bootstrap seed set - explicit seeds and the
    /// global config's DHT static nodes - so growth can resolve member
    /// `address` values through nodes guaranteed to answer `dht.findValue`.
    /// See [`udp_peer_growth_with_resolvers`].
    growth_resolvers: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    /// Overlay member endpoints shared with the UDP lookups and the QUIC
    /// discovery path.
    ///
    /// The UDP lookups record every resolved member's ADNL UDP endpoint (and
    /// any advertised `adnl.address.quic`); QUIC discovery probes those
    /// members, because the bootstrap `dht.static_nodes` run `dht-server`,
    /// which has no QUIC listener.  See [`crate::OverlayMemberEndpoints`].
    member_endpoints: crate::OverlayMemberEndpoints,
    overlay_max_peers: u32,
    external_address: Option<std::net::SocketAddr>,
    address_publisher: Option<DhtAddressPublisher>,
    /// Local ADNL key, remembered by the transport configurators
    /// so `start` can sign overlay broadcasts with it.
    local_keypair: Option<KeyPair>,
    /// Fixed ADNL address lists (seeds, validators) that bootstrap with
    /// reconnect support, are never evicted, and never count against
    /// [`overlay_max_peers`](Self::overlay_max_peers).
    ///
    /// See [`pinned_peers`](Self::pinned_peers).
    pinned_peers: Vec<SeedPeer>,
    /// Isolated cap on connected pinned peers; `None` connects the whole
    /// pinned list.
    pinned_max_peers: Option<u32>,
    /// Delivery-quality thresholds for attribution, eviction, and send
    /// ranking.
    peer_quality: PeerQualityConfig,
    /// Whether the background churn task evicts the worst non-pinned peers
    /// over [`overlay_max_peers`](Self::overlay_max_peers).
    quality_eviction: bool,
    /// Durable peer-store file; `None` keeps startup behavior unchanged.
    peer_store_path: Option<std::path::PathBuf>,
    /// Interval between automatic peer-store saves.
    peer_store_save_interval: Duration,
    /// Interval between background discovery rounds while the store runs.
    peer_store_discovery_interval: Duration,
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
            .field("peer_growth_lookup", &self.peer_growth_lookup.is_some())
            .field("growth_resolvers", &self.growth_resolvers)
            .field(
                "member_endpoints",
                &crate::registered_member_count(&self.member_endpoints),
            )
            .field("overlay_max_peers", &self.overlay_max_peers)
            .field("external_address", &self.external_address)
            .field("address_publisher", &self.address_publisher.is_some())
            .field("local_keypair", &self.local_keypair.is_some())
            .field("pinned_peers", &self.pinned_peers)
            .field("pinned_max_peers", &self.pinned_max_peers)
            .field("peer_quality", &self.peer_quality)
            .field("quality_eviction", &self.quality_eviction)
            .field("peer_store_path", &self.peer_store_path)
            .field("peer_store_save_interval", &self.peer_store_save_interval)
            .field(
                "peer_store_discovery_interval",
                &self.peer_store_discovery_interval,
            )
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
            overlay_id: OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
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
            peer_growth_lookup: None,
            growth_resolvers: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            member_endpoints: crate::OverlayMemberEndpoints::default(),
            overlay_max_peers: 30,
            external_address: None,
            address_publisher: None,
            local_keypair: None,
            pinned_peers: Vec::new(),
            pinned_max_peers: None,
            peer_quality: PeerQualityConfig::default(),
            quality_eviction: false,
            peer_store_path: None,
            peer_store_save_interval: crate::peer_store::DEFAULT_PEER_STORE_SAVE_INTERVAL,
            peer_store_discovery_interval:
                crate::discovery_task::DEFAULT_PEER_STORE_DISCOVERY_INTERVAL,
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
        let external_address = self.external_address;
        // The overlay-node DHT value is this overlay's full-node
        // phonebook: every entry can verify this node and then broadcast
        // to it, so bootstrap discovery resolves as many entries as the
        // session pool can hold instead of a small fixed sample.
        // `overlay_max_peers(0)` disables managed discovery entirely.
        let discovery_records = usize::try_from(self.overlay_max_peers).unwrap_or(usize::MAX);
        let session_factory = overlay_factory_with_endpoints(
            local_addr,
            local_keypair,
            overlay,
            channel_timeout,
            self.member_endpoints.clone(),
        );
        let mut builder = self.session_factory(session_factory);
        // The transport configurators remember the local key so
        // `start` can sign overlay broadcasts with it.
        builder.local_keypair = Some(local_keypair);
        // Cloned before the call below moves `builder` into its receiver.
        let member_endpoints = builder.member_endpoints.clone();
        builder = match builder.dht_overlay_key {
            Some(overlay_key) => builder.seed_discovery_lookup(udp_overlay_lookup_with_endpoints(
                local_addr,
                local_keypair,
                overlay,
                overlay_key,
                discovery_records,
                discovery_timeout,
                member_endpoints,
            )),
            None => builder,
        };
        builder.peer_growth_lookup = Some(udp_peer_growth_with_resolvers(
            local_addr,
            local_keypair,
            overlay,
            discovery_timeout,
            Arc::clone(&builder.growth_resolvers),
            builder.member_endpoints.clone(),
        ));
        builder.address_publisher = Some(address_publisher(
            local_addr,
            local_keypair,
            external_address,
            discovery_timeout,
        ));
        builder
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
        let overlay_id = OverlayId::from_shard_public(workchain, shard, zero_state_file_hash);
        self.dht_overlay_key(overlay_id.as_bytes()).native_udp(
            local_addr,
            local_keypair,
            channel_timeout,
        )
    }

    /// Configures QUIC-based overlay sessions for peer connections.
    ///
    /// With a configured [`Self::dht_overlay_key`] the builder also installs
    /// the QUIC-backed seed discovery ([`quic_overlay_lookup_with_endpoints`]),
    /// mirroring what [`Self::native_udp`] does for direct ADNL/UDP sessions.
    ///
    /// Discovery and sessions prefer the overlay members registered in
    /// [`Self::member_endpoints`] over the bootstrap seeds: upstream runs
    /// QUIC in full nodes, not in `dht-server`, so only members can answer.
    pub fn native_quic(self, local_addr: std::net::SocketAddr, local_keypair: KeyPair) -> Self {
        let discovery_timeout = self.discovery_timeout;
        let overlay = self.overlay_id;
        let endpoints = self.member_endpoints.clone();
        let session_factory = quic_overlay_factory_with_endpoints(
            local_addr,
            local_keypair,
            overlay,
            endpoints.clone(),
        );
        let mut builder = self.session_factory(session_factory);
        builder.local_keypair = Some(local_keypair);
        match builder.dht_overlay_key {
            Some(overlay_key) => builder.seed_discovery_lookup(quic_overlay_lookup_with_endpoints(
                local_addr,
                local_keypair,
                overlay,
                overlay_key,
                16,
                discovery_timeout,
                endpoints,
            )),
            None => builder,
        }
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
        let discovery_timeout = self.discovery_timeout;
        let external_address = self.external_address;
        let session_factory = overlay_factory_with_endpoints(
            local_addr,
            local_keypair,
            self.overlay_id,
            channel_timeout,
            self.member_endpoints.clone(),
        );
        let mut builder = self.session_factory(session_factory);
        builder.local_keypair = Some(local_keypair);
        // Explicit seeds are still DHT nodes, so publishing here lets the
        // seeds resolve this node's address back, same as on the full
        // native UDP path.
        builder.address_publisher = Some(address_publisher(
            local_addr,
            local_keypair,
            external_address,
            discovery_timeout,
        ));
        builder
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

    /// Sets the target number of concurrently connected overlay members.
    ///
    /// Peer growth stops once the pool holds this many sessions.  The default
    /// of 30 matches pytoniq's `OverlayManager(max_peers=30)`; upstream's
    /// admission gate is a per-member pending queue that drains 60 nodes a
    /// minute, so the odds of being pinged - and therefore of ever receiving a
    /// broadcast - grow roughly linearly with this number.
    ///
    /// A value of `0` disables periodic peer growth entirely; with
    /// [`quality_eviction`](Self::quality_eviction) enabled it still caps the
    /// managed set at zero, evicting every non-pinned session.
    pub fn overlay_max_peers(mut self, max_peers: u32) -> Self {
        self.overlay_max_peers = max_peers;
        self
    }

    /// Pins a fixed ADNL address list (seeds, validators) to this scanner.
    ///
    /// Pinned peers bootstrap like explicit seeds - with reconnect support -
    /// but are additionally excluded from [`overlay_max_peers`](Self::overlay_max_peers)
    /// (they are a separate, guaranteed set), are never re-added or evicted by
    /// the background peer tasks, and survive quality churn even with a
    /// terrible delivery score.
    pub fn pinned_peers(mut self, peers: impl IntoIterator<Item = SeedPeer>) -> Self {
        self.pinned_peers.extend(peers);
        self
    }

    /// Caps how many pinned peers are connected at once - an isolated limit,
    /// separate from [`overlay_max_peers`](Self::overlay_max_peers).
    ///
    /// The first `limit` entries of [`pinned_peers`](Self::pinned_peers) are
    /// connected; the rest are dropped at `start`. `None` (the default)
    /// connects the whole pinned list.
    pub fn pinned_max_peers(mut self, limit: Option<u32>) -> Self {
        self.pinned_max_peers = limit;
        self
    }

    /// Overrides the delivery-quality configuration: late threshold, miss
    /// window, eviction protection, sweep and churn cadence, score decay.
    pub fn peer_quality(mut self, config: PeerQualityConfig) -> Self {
        self.peer_quality = config;
        self
    }

    /// Enables background eviction of the worst non-pinned peers whenever the
    /// managed connection count exceeds [`overlay_max_peers`](Self::overlay_max_peers).
    ///
    /// Disabled by default so existing setups keep their connection set;
    /// quality metrics and send ranking are always collected, and only peers
    /// with at least [`PeerQualityConfig::min_observations`] observations are
    /// ever evicted.
    ///
    /// With the builder's own wiring this option rarely fires: bootstrap and
    /// the peer-growth task admit at most
    /// [`overlay_max_peers`](Self::overlay_max_peers) managed sessions, and
    /// only a lost peer's reconnection re-registering after growth filled
    /// its slot (or a custom session factory adding managed peers) can
    /// push the roster over the cap - and even then only peers with enough
    /// delivery observations are eligible.  The quality observability
    /// (`snapshot`, `evictions`) works either way.
    pub fn quality_eviction(mut self, enabled: bool) -> Self {
        self.quality_eviction = enabled;
        self
    }

    /// Sets the externally reachable UDP address published in the
    /// DHT `address` value for this node's ADNL id.
    ///
    /// Overlay peers that learn this node's signed record from an
    /// `overlay.getRandomPeers` answer resolve its address through
    /// that value, so only nodes that publish it can be reached by
    /// members they never contacted - which is what makes transitive
    /// peer discovery compound.  When unset the scanner falls back to
    /// the `TON_MEMPOOL_EXTERNAL_ADDRESS` environment variable and
    /// then to a route probe against the configured seeds; the probe
    /// only yields an address on a globally routable host, so behind
    /// NAT an explicit address is required.
    pub fn external_address(mut self, address: std::net::SocketAddr) -> Self {
        self.external_address = Some(address);
        self
    }

    /// Installs the periodic peer-growth lookup used after bootstrap.
    ///
    /// [`native_udp`](Self::native_udp) wires this up automatically with
    /// [`udp_peer_growth`]; set it manually only for a custom session factory.
    pub fn peer_growth_lookup(mut self, lookup: SeedDiscoveryLookup) -> Self {
        self.peer_growth_lookup = Some(lookup);
        self
    }

    /// Shares an overlay member endpoint registry with this builder.
    ///
    /// [`native_udp`](Self::native_udp) records every member it resolves -
    /// and any advertised `adnl.address.quic` - into the builder's own
    /// registry, while [`native_quic`](Self::native_quic) probes registered
    /// members instead of the bootstrap seeds.  Set the same registry on
    /// several builders to hand a UDP-discovered member set to a QUIC
    /// scanner; the default is a fresh, empty registry per builder.
    pub fn member_endpoints(mut self, endpoints: crate::OverlayMemberEndpoints) -> Self {
        self.member_endpoints = endpoints;
        self
    }

    /// Enables durable peer persistence at `path` (a versioned JSON file,
    /// format documented on [`PeerStore`]).
    ///
    /// The file is read at startup and rewritten after bootstrap, on every
    /// save interval ([`peer_store_save_interval`](Self::peer_store_save_interval)),
    /// and on shutdown:
    ///
    /// - every peer ever learned stays persisted - connected, disconnected,
    ///   failed, or evicted - with its UDP/QUIC endpoints and a persistent
    ///   `priority` that this run corrects (fast handshakes and delivered
    ///   broadcasts raise it, failed dials and dropped sessions lower it,
    ///   untouched peers decay);
    /// - a **warm** file (at least one dialable entry) joins the bootstrap
    ///   dial set in priority order and skips the blocking start-time
    ///   discovery lookup; background rounds
    ///   ([`peer_store_discovery_interval`](Self::peer_store_discovery_interval))
    ///   keep discovering fresher members while the event queue is idle;
    /// - if every warm-store dial fails at bootstrap, the skipped blocking
    ///   discovery runs once as a fallback before startup gives up;
    /// - a cold, unreadable, or foreign file (other `version`, `network`, or
    ///   `overlay`) starts empty and startup behaves exactly as without a
    ///   store.
    pub fn peer_store(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.peer_store_path = Some(path.into());
        self
    }

    /// Interval between automatic peer-store saves (default 60 s).  A save
    /// also runs right after bootstrap and once on shutdown.
    pub fn peer_store_save_interval(mut self, interval: Duration) -> Self {
        self.peer_store_save_interval = interval;
        self
    }

    /// Interval between background discovery rounds while the store is
    /// enabled (default 60 s).  On a warm store these rounds replace the
    /// blocking start-time lookup and keep searching for better peers; a
    /// round additionally requires an idle event queue.
    pub fn peer_store_discovery_interval(mut self, interval: Duration) -> Self {
        self.peer_store_discovery_interval = interval;
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
        let seed_count = seeds.len();
        // Pinned peers are validated up front and merged into the bootstrap
        // set below; they must not be silently dropped by a failed lookup.
        let mut pinned = self.pinned_peers.clone();
        if let Some(limit) = self.pinned_max_peers {
            pinned.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        for peer in &pinned {
            if !peer.is_valid() {
                return Err(MempoolError::InvalidBootstrapAddress(peer.address.clone()));
            }
        }
        let pinned_ids = pinned.iter().map(|peer| peer.peer).collect::<HashSet<_>>();
        // Hand the bootstrap seed set to the peer-growth lookup: explicit
        // seeds and the config's DHT static nodes are DHT nodes, so growth
        // resolves member `address` values through nodes that answer
        // `dht.findValue`, the same role pytoniq's DhtClient node set plays.
        *self.growth_resolvers.write().await = seeds.clone();
        let discovery = DiscoveryConfig {
            overlay: self.overlay_id,
            seeds: seeds.clone(),
            lookup_timeout: self.discovery_timeout,
            max_records: 64,
        };
        // The configured lookup, normalized across the three builder
        // variants; the same runner backs the warm-store fallback and the
        // background discovery rounds.
        let runner = discovery_runner(
            self.seed_discovery_lookup.clone(),
            self.typed_discovery_lookup.clone(),
            self.discovery_lookup.clone(),
            self.overlay_id,
            self.discovery_timeout,
        );
        // Durable peer memory.  A warm store (at least one dialable entry)
        // joins the dial set below and skips the blocking start-time lookup;
        // background discovery rounds take over for the rest of the run.
        let store = self.peer_store_path.clone().map(|path| {
            PeerStore::load(
                path,
                if self.testnet { "testnet" } else { "mainnet" },
                &hex::encode(self.overlay_id.as_bytes()),
            )
        });
        let saved_all = match &store {
            Some(store) => store.seeds().await,
            None => Vec::new(),
        };
        let warm = !saved_all.is_empty();
        // Bootstrap dials at most twice the managed budget from the store
        // (never fewer than 16); the remaining entries stay in the
        // known-candidate list, where background rounds dial them by priority
        // under the budget instead of all at once.
        let store_budget = usize::try_from(self.overlay_max_peers)
            .unwrap_or(usize::MAX)
            .saturating_mul(2)
            .max(16);
        let saved = saved_all
            .iter()
            .take(store_budget)
            .cloned()
            .collect::<Vec<_>>();
        if warm {
            log::info!(
                "mempool::start: peer store warm with {} peer(s), dialing up to {store_budget} and skipping the blocking discovery",
                saved_all.len()
            );
        }
        let discovered = if warm {
            Vec::new()
        } else if let Some(runner) = &runner {
            let found = runner(seeds.clone()).await;
            if found.is_empty() {
                log::debug!(
                    "discovery: timed out or empty, falling back to the {} bootstrap seed(s)",
                    seeds.len()
                );
            } else {
                log::debug!("discovery: found {} overlay member(s)", found.len());
            }
            found
        } else {
            discovery.discover(|| async { Vec::new() }).await
        };
        let discovered_count = discovered.len();
        // Membership comes from the warm store or from discovery; the seed
        // set is kept only as the bootstrap fallback when neither yielded
        // anything to talk to (the seeds are `dht-server` contacts without
        // overlay sessions - TODO item 411).
        let members = if warm {
            saved
        } else if discovered.is_empty() {
            seeds.clone()
        } else {
            discovered
        };
        // Merge the pinned set first: pinned sessions are attempted (and
        // remembered) ahead of member results, deduplicated by peer id so a
        // pinned address wins over a discovered or stored one.
        let peers = merge_peer_sets([pinned.as_slice(), members.as_slice()]);
        // Candidate list shared with growth, reconnect, and the background
        // rounds: a warm store keeps its entire loaded set here, beyond the
        // bootstrap dial budget, so those peers stay reachable later.
        let known_init = if warm {
            merge_peer_sets([pinned.as_slice(), saved_all.as_slice()])
        } else {
            peers.clone()
        };
        if peers.is_empty() {
            return Err(MempoolError::NoBootstrapPeers);
        }
        log::info!(
            "mempool::start: seeds={} discovery_results={} peers={} attempting {} session(s)",
            seed_count,
            discovered_count,
            peers.len(),
            peers.len()
        );
        let manager = PeerManager::with_overlay(self.overlay, self.overlay_id)
            .map_err(|error| MempoolError::Overlay(error.to_string()))?;
        let growth_interval = self.config.growth_interval;
        let scanner = Arc::new(MempoolScanner::with_quality(
            self.config,
            self.peer_quality.clone(),
        )?);
        if let Some(keypair) = self.local_keypair {
            scanner
                .add_broadcast_peer(Arc::new(
                    OverlayBroadcastPeer::new(Arc::new(manager.clone()), keypair)
                        .with_quality(scanner.quality()),
                ))
                .await;
            log::debug!("mempool broadcast: overlay-direct send path armed");
        }
        // Quality metrics run in every scanner; the churn task only starts
        // when eviction is explicitly enabled.
        crate::peer_quality::spawn_quality_tasks(
            scanner.quality(),
            manager.clone(),
            pinned_ids.clone(),
            self.quality_eviction
                .then_some(usize::try_from(self.overlay_max_peers).unwrap_or(usize::MAX)),
        );
        scanner.record_discovery(DiscoveryStats {
            seeds: seed_count as u64,
            discovered: discovered_count as u64,
            peers: peers.len() as u64,
        });
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
            let known_peers: Arc<tokio::sync::RwLock<Vec<SeedPeer>>> =
                Arc::new(tokio::sync::RwLock::new(known_init));
            let reconnect_factory: ReconnectFactory = {
                let factory = factory.clone();
                let known = known_peers.clone();
                Arc::new(move |peer| {
                    let factory = factory.clone();
                    let known = known.clone();
                    Box::pin(async move {
                        let seed = known.read().await.iter().find(|s| s.peer == peer).cloned();
                        let seed = seed
                            .ok_or_else(|| format!("peer {peer:?} not found in known peer list"))?;
                        factory(seed).await
                    })
                })
            };
            let dial = BootstrapDial {
                factory: &factory,
                manager: &manager,
                reconnect: &reconnect_factory,
                attempts: self.reconnect_attempts,
                backoff: self.reconnect_backoff,
                store: store.as_ref(),
            };
            let (mut factory_successes, factory_failures) = dial.dial(&peers).await;
            log::info!(
                "mempool bootstrap: factory_successes={} factory_failures={}",
                factory_successes,
                factory_failures
            );
            if factory_successes == 0 && warm {
                // Every warm-store candidate went stale: run the blocking
                // discovery the warm start skipped and retry with the fresh
                // members before giving up on bootstrap.
                if let Some(runner) = &runner {
                    factory_successes = warm_store_fallback(
                        runner,
                        &seeds,
                        &pinned,
                        &saved_all,
                        &known_peers,
                        &dial,
                    )
                    .await
                    .0;
                }
            }
            if factory_successes == 0 {
                manager.shutdown();
                return Err(MempoolError::Session(
                    "all validated bootstrap sessions failed".into(),
                ));
            }
            await_registered(&manager, self.bootstrap_timeout).await?;
            if let Some(growth) = self.peer_growth_lookup.clone() {
                PeerGrowthTask {
                    manager: manager.clone(),
                    growth,
                    known: known_peers.clone(),
                    factory: factory.clone(),
                    reconnect: reconnect_factory.clone(),
                    attempts: self.reconnect_attempts,
                    backoff: self.reconnect_backoff,
                    max_peers: usize::try_from(self.overlay_max_peers).unwrap_or(usize::MAX),
                    pinned: pinned_ids.clone(),
                    growth_interval,
                    event_tx: scanner.event_tx.clone(),
                    store: store.clone(),
                }
                .spawn();
            }
            if let Some(publish) = self.address_publisher.clone() {
                let manager = manager.clone();
                let known = known_peers.clone();
                let mut shutdown = manager.subscribe_shutdown();
                tokio::spawn(async move {
                    // Publish before the first growth round so
                    // transitive discovery can resolve this node's
                    // address right away, then re-publish inside
                    // the value's one hour lifetime.  The first
                    // round is staggered over a random window:
                    // parallel scanners behind one NAT fire their
                    // STUN probes and store rounds in the same
                    // second, which correlates the candidate
                    // silence the retries exist for (TODO item
                    // 447).
                    let stagger_ms = (rand::random::<f64>()
                        * f64::from(
                            u32::try_from(PUBLISH_FIRST_ROUND_STAGGER_MAX.as_millis())
                                .unwrap_or(u32::MAX),
                        )) as u64;
                    tokio::select! {
                        _ = shutdown.changed() => return,
                        () = tokio::time::sleep(Duration::from_millis(stagger_ms)) => {}
                    }
                    loop {
                        let nodes = known.read().await.clone();
                        if !nodes.is_empty() {
                            publish(nodes).await;
                        }
                        tokio::select! {
                            _ = shutdown.changed() => return,
                            () = tokio::time::sleep(PUBLISH_INTERVAL) => {}
                        }
                    }
                });
            }
            // Durable save loop: an immediate bootstrap snapshot, then every
            // save interval, and one final write on shutdown.
            if let Some(store) = store.clone() {
                PeerStoreWriter {
                    store,
                    known: known_peers.clone(),
                    endpoints: self.member_endpoints.clone(),
                    manager: manager.clone(),
                    quality: scanner.quality(),
                    interval: self.peer_store_save_interval,
                }
                .spawn();
            }
            // Background replacement for the skipped start-time discovery:
            // idle-time rounds that merge fresher members and dial known-but-
            // down peers in persisted-priority order under the budget.
            if let (Some(runner), Some(store)) = (runner.clone(), store.clone()) {
                PeerDiscoveryTask {
                    manager: manager.clone(),
                    runner,
                    seeds: seeds.clone(),
                    known: known_peers.clone(),
                    factory: factory.clone(),
                    reconnect: reconnect_factory.clone(),
                    attempts: self.reconnect_attempts,
                    backoff: self.reconnect_backoff,
                    max_peers: usize::try_from(self.overlay_max_peers).unwrap_or(usize::MAX),
                    pinned: pinned_ids.clone(),
                    interval: self.peer_store_discovery_interval,
                    event_tx: scanner.event_tx.clone(),
                    store: Some(store),
                }
                .spawn();
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
