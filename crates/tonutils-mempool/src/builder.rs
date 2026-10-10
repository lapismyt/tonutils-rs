//! `MempoolScannerBuilder`: bootstrap resolution and startup wiring.
//!
//! Split out of `lib.rs` to keep that file near the repository's 1000 line
//! limit; `lib.rs` re-exports [`MempoolScannerBuilder`].

use super::*;

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
    overlay_max_peers: u32,
    external_address: Option<std::net::SocketAddr>,
    address_publisher: Option<DhtAddressPublisher>,
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
            .field("overlay_max_peers", &self.overlay_max_peers)
            .field("external_address", &self.external_address)
            .field("address_publisher", &self.address_publisher.is_some())
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
            overlay_max_peers: 30,
            external_address: None,
            address_publisher: None,
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
        let session_factory = overlay_factory(local_addr, local_keypair, overlay, channel_timeout);
        let mut builder = self.session_factory(session_factory);
        builder = match builder.dht_overlay_key {
            Some(overlay_key) => builder.seed_discovery_lookup(udp_overlay_lookup(
                local_addr,
                local_keypair,
                overlay,
                overlay_key,
                16,
                discovery_timeout,
            )),
            None => builder,
        };
        builder.peer_growth_lookup = Some(udp_peer_growth(
            local_addr,
            local_keypair,
            overlay,
            discovery_timeout,
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
    /// the QUIC-backed seed discovery ([`quic_overlay_lookup`]), mirroring what
    /// [`Self::native_udp`] does for direct ADNL/UDP sessions.
    pub fn native_quic(self, local_addr: std::net::SocketAddr, local_keypair: KeyPair) -> Self {
        let discovery_timeout = self.discovery_timeout;
        let overlay = self.overlay_id;
        let session_factory = quic_overlay_factory(local_addr, local_keypair, overlay);
        let builder = self.session_factory(session_factory);
        match builder.dht_overlay_key {
            Some(overlay_key) => builder.seed_discovery_lookup(quic_overlay_lookup(
                local_addr,
                local_keypair,
                overlay,
                overlay_key,
                16,
                discovery_timeout,
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
        let session_factory =
            overlay_factory(local_addr, local_keypair, self.overlay_id, channel_timeout);
        let mut builder = self.session_factory(session_factory);
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
    /// A value of `0` disables periodic peer growth entirely.
    pub fn overlay_max_peers(mut self, max_peers: u32) -> Self {
        self.overlay_max_peers = max_peers;
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
        let discovery = DiscoveryConfig {
            overlay: self.overlay_id,
            seeds: seeds.clone(),
            lookup_timeout: self.discovery_timeout,
            max_records: 64,
        };
        let peers = if let Some(lookup) = self.seed_discovery_lookup.clone() {
            log::debug!(
                "seed_discovery_lookup: starting with {} seeds, timeout={:?}",
                seeds.len(),
                self.discovery_timeout
            );
            let result = tokio::time::timeout(self.discovery_timeout, lookup(seeds.clone()))
                .await
                .ok()
                .filter(|peers| !peers.is_empty());
            let mut peers = result.unwrap_or_default();
            let discovered_count = peers.len();
            let mut seen = peers
                .iter()
                .map(|peer| (peer.peer, peer.address.clone()))
                .collect::<std::collections::HashSet<_>>();
            for seed in &seeds {
                if seen.insert((seed.peer, seed.address.clone())) {
                    peers.push(seed.clone());
                }
            }
            if discovered_count == 0 {
                log::debug!(
                    "seed_discovery_lookup: timed out or empty, using {} raw seeds",
                    seeds.len()
                );
            } else {
                log::debug!(
                    "seed_discovery_lookup: found {discovered_count} overlay peers and retained {} raw seeds",
                    peers.len().saturating_sub(discovered_count)
                );
            }
            peers
        } else if let Some(lookup) = self.typed_discovery_lookup.clone() {
            discovery.discover_typed(move |seeds| lookup(seeds)).await
        } else if let Some(lookup) = self.discovery_lookup.clone() {
            discovery.discover_with(move |seeds| lookup(seeds)).await
        } else {
            discovery.discover(|| async { Vec::new() }).await
        };
        let discovered_count = peers.len().saturating_sub(seed_count);
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
        let scanner = Arc::new(MempoolScanner::new(self.config)?);
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
                Arc::new(tokio::sync::RwLock::new(peers.clone()));
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
            let results = join_all(peers.into_iter().map(|peer| {
                let factory = factory.clone();
                async move {
                    log::debug!(
                        "mempool bootstrap: attempting peer={:?} address={}",
                        peer.peer,
                        peer.address
                    );
                    let result = factory(peer.clone()).await;
                    match &result {
                        Ok(_) => log::debug!(
                            "mempool bootstrap: direct overlay session established for peer={:?} address={}",
                            peer.peer,
                            peer.address
                        ),
                        Err(error) => log::warn!(
                            "mempool bootstrap: peer={:?} address={} failed: {error}",
                            peer.peer,
                            peer.address
                        ),
                    }
                    result
                }
            }))
            .await;
            let mut factory_successes = 0;
            let mut factory_failures = 0;
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
                        factory_successes += 1;
                    }
                    Err(error) => {
                        factory_failures += 1;
                        log::warn!("bootstrap session failed: {error}");
                    }
                }
            }
            log::info!(
                "mempool bootstrap: factory_successes={} factory_failures={}",
                factory_successes,
                factory_failures
            );
            if factory_successes == 0 {
                manager.shutdown();
                return Err(MempoolError::Session(
                    "all validated bootstrap sessions failed".into(),
                ));
            }
            if self.bootstrap_timeout != Duration::ZERO {
                let deadline = tokio::time::Instant::now() + self.bootstrap_timeout;
                while manager.peer_count().await == 0 {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        manager.shutdown();
                        return Err(MempoolError::Session(
                            "bootstrap sessions did not register any peers".into(),
                        ));
                    }
                    tokio::time::sleep(remaining.min(Duration::from_millis(10))).await;
                }
                log::info!(
                    "mempool bootstrap ready: registered_peers={}",
                    manager.peer_count().await
                );
            }
            if let Some(growth) = self.peer_growth_lookup.clone()
                && self.overlay_max_peers > 0
            {
                let manager = manager.clone();
                let known = known_peers.clone();
                let factory = factory.clone();
                let reconnect = reconnect_factory.clone();
                let attempts = self.reconnect_attempts;
                let backoff = self.reconnect_backoff;
                let max_peers = usize::try_from(self.overlay_max_peers).unwrap_or(usize::MAX);
                let mut shutdown = manager.subscribe_shutdown();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.changed() => return,
                            () = tokio::time::sleep(PEER_GROWTH_INTERVAL) => {}
                        }
                        if manager.peer_count().await >= max_peers {
                            continue;
                        }
                        let seeds = known.read().await.clone();
                        if seeds.is_empty() {
                            continue;
                        }
                        // pytoniq parity: every round announces to as
                        // many members as are needed to fill the pool
                        // (`dif = max_peers - registered`), not to one.
                        // Each lookup asks the next member in round-robin
                        // order, so `dif` concurrent lookups announce to
                        // `dif` distinct members - the fan-out that makes
                        // transitive discovery compound.
                        let dif = max_peers.saturating_sub(manager.peer_count().await).max(1);
                        let rounds = dif.min(seeds.len());
                        let results = join_all((0..rounds).map(|_| growth(seeds.clone()))).await;
                        let candidates = results.into_iter().flatten().collect::<Vec<_>>();
                        let mut added = 0usize;
                        let mut discovered = Vec::new();
                        for candidate in candidates {
                            if manager.peer_count().await >= max_peers {
                                break;
                            }
                            if known
                                .read()
                                .await
                                .iter()
                                .any(|seed| seed.peer == candidate.peer)
                            {
                                continue;
                            }
                            match factory(candidate.clone()).await {
                                Ok(session) => {
                                    manager
                                        .add_session_with_reconnect(
                                            session,
                                            reconnect.clone(),
                                            attempts,
                                            backoff,
                                        )
                                        .await;
                                    discovered.push(candidate);
                                    added += 1;
                                }
                                Err(error) => log::debug!(
                                    "peer growth: session to {} failed: {error}",
                                    candidate.address
                                ),
                            }
                        }
                        if !discovered.is_empty() {
                            known.write().await.extend(discovered);
                        }
                        if added > 0 {
                            log::info!(
                                "peer growth: added {added} session(s), registered_peers={}",
                                manager.peer_count().await
                            );
                        }
                    }
                });
            }
            if let Some(publish) = self.address_publisher.clone() {
                let manager = manager.clone();
                let known = known_peers.clone();
                let mut shutdown = manager.subscribe_shutdown();
                tokio::spawn(async move {
                    // Publish before the first growth round so
                    // transitive discovery can resolve this node's
                    // address right away, then re-publish inside
                    // the value's one hour lifetime.
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
