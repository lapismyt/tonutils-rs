//! Durable cross-run peer memory: [`PeerStore`] keeps every peer the overlay
//! has ever learned - connected, disconnected, failed, or evicted - inside a
//! versioned JSON file, together with a persistent dial priority that is
//! corrected by observed reliability and speed and survives restarts.
//!
//! The builder wires the store through
//! [`crate::overlay::OverlayNodeBuilder::peer_store`]:
//! at startup the file's peers join the bootstrap dial set (a warm file skips
//! the blocking discovery lookup entirely; the background discovery task keeps
//! hunting for fresher members in the background), and a background writer
//! rewrites the file after bootstrap, on every save interval, and on shutdown.
//!
//! File format (`version` 1):
//!
//! ```json
//! {
//!   "version": 1,
//!   "network": "mainnet",
//!   "overlay": "<32-byte hex overlay id>",
//!   "saved_at": 1792600000,
//!   "peers": [
//!     {
//!       "id": "<32-byte hex peer id>",
//!       "address": "1.2.3.4:32700",
//!       "last_seen": 1792599000,
//!       "source": "growth",
//!       "status": "disconnected",
//!       "priority": 0.612,
//!       "udp": "1.2.3.4:32700",
//!       "quic": "1.2.3.4:33700"
//!     }
//!   ]
//! }
//! ```
//!
//! Load-time filters: a file whose `version`, `network`, or `overlay` does not
//! match is ignored with a warning; entries without any usable dial address
//! (neither `address` nor `udp`) are dropped; duplicate ids keep the entry
//! with the higher `(priority, last_seen)`; the list is priority-sorted and
//! capped at [`MAX_STORED_PEERS`].  Each load also applies one per-run decay
//! pass (see [`RUN_DECAY`]), so peers that do nothing across runs slowly sink
//! instead of holding a stale rank forever.

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    overlay::{OverlayMemberEndpoints, PeerQualityRegistry},
    overlay::{PeerId, PeerManager, PeerStatus, SeedPeer},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// On-disk format version written by this build.
///
/// A file with any other `version` is ignored entirely: the format is not
/// parsed best-effort across versions.
pub const PEER_STORE_VERSION: u32 = 1;

/// Hard cap on persisted peers; load and save both truncate to it, keeping
/// the highest-priority entries.
pub const MAX_STORED_PEERS: usize = 512;

/// Default interval between automatic saves.  The writer additionally saves
/// once immediately after bootstrap and once more on shutdown, so at most one
/// interval of experience is lost on a hard kill.
pub const DEFAULT_PEER_STORE_SAVE_INTERVAL: Duration = Duration::from_secs(60);

/// Priority of a peer with no recorded history.
const DEFAULT_PRIORITY: f64 = 0.5;
/// Lower clamp: a peer stays a candidate (ranked last) even after a bad run.
const MIN_PRIORITY: f64 = 0.05;
/// Upper clamp: one excellent run must not bury the rest of the roster.
const MAX_PRIORITY: f64 = 1.0;
/// Bonus for an established dial, scaled by handshake speed
/// (see [`speed_factor`]): reliability and latency both pay.
const DIAL_OK_BONUS: f64 = 0.05;
/// Penalty for a dial that never produced a session.
const DIAL_FAIL_PENALTY: f64 = 0.1;
/// Penalty for a session that dropped or failed after establishment.
///
/// A quality-churn eviction surfaces as the same `Disconnected` status, so it
/// costs this penalty too; the status events carry no cause, so cause-specific
/// weights are intentionally not double-counted here.
const DISCONNECT_PENALTY: f64 = 0.1;
/// One-time (per run) bonus for a peer that delivered at least one broadcast,
/// credited from the [`PeerQualityRegistry`] snapshot at save time.
const DELIVERY_BONUS: f64 = 0.1;
/// Multiplicative decay applied once per load: a peer untouched for `n` runs
/// keeps `DEFAULT_PRIORITY * RUN_DECAY^n`, slowly sinking below active peers.
const RUN_DECAY: f64 = 0.98;

/// Entry `status` values.  The field is an open string on load - foreign
/// writers keep whatever labels they use - but this crate only writes these.
pub const STATUS_CONNECTED: &str = "connected";
/// See [`STATUS_CONNECTED`].
pub const STATUS_DISCONNECTED: &str = "disconnected";
/// See [`STATUS_CONNECTED`].
pub const STATUS_FAILED: &str = "failed";
/// See [`STATUS_CONNECTED`].
pub const STATUS_UNKNOWN: &str = "unknown";

/// Entry `source` label for peers discovered by the background task or the
/// endpoint registry.  Loaded entries keep the source recorded by the run
/// that learned them.
pub const SOURCE_DISCOVERY: &str = "discovery";
/// See [`SOURCE_DISCOVERY`].
pub const SOURCE_GROWTH: &str = "growth";
/// See [`SOURCE_DISCOVERY`].
pub(crate) const SOURCE_BOOTSTRAP: &str = "bootstrap";

/// Handshake speed factor for the dial-success bonus: a sub-100 ms connect is
/// a nearby, healthy peer (1.0), a sub-second one is average (0.5), and a
/// slow one only proves reachability (0.25).
fn speed_factor(latency: Duration) -> f64 {
    if latency < Duration::from_millis(100) {
        1.0
    } else if latency < Duration::from_secs(1) {
        0.5
    } else {
        0.25
    }
}

/// Clamps into `[MIN_PRIORITY, MAX_PRIORITY]` and rounds to three decimals
/// (stable file diffs).  Non-finite input sanitizes to the default first, so
/// `f64::clamp` can never see a NaN bound or payload.
fn clamp_priority(priority: f64) -> f64 {
    let finite = if priority.is_finite() {
        priority
    } else {
        DEFAULT_PRIORITY
    };
    (finite.clamp(MIN_PRIORITY, MAX_PRIORITY) * 1000.0).round() / 1000.0
}

/// Whether an address string is dialable: the same rule as
/// [`SeedPeer::is_valid`] minus the peer id.
fn usable_address(address: &str) -> bool {
    address.len() <= 256
        && address
            .parse::<std::net::SocketAddr>()
            .is_ok_and(|address| address.port() != 0 && !address.ip().is_unspecified())
}

fn default_source() -> String {
    STATUS_UNKNOWN.to_owned()
}

fn default_priority() -> f64 {
    DEFAULT_PRIORITY
}

/// One persisted peer.  `id` is hex-encoded in JSON; `udp`/`quic` carry the
/// endpoint-registry extras so QUIC probing and growth can be re-seeded
/// without a fresh DHT address lookup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredPeer {
    /// Stable overlay peer id.
    #[serde(with = "peer_id_hex")]
    pub id: PeerId,
    /// Primary dial address (the most recent one acted on).
    pub address: String,
    /// Unix seconds when the scanner last heard from this peer.
    #[serde(default)]
    pub last_seen: u64,
    /// Where the entry originated (`growth`, `discovery`, ...); preserved
    /// across runs for loaded entries.
    #[serde(default = "default_source")]
    pub source: String,
    /// Last observed lifecycle status; disconnected and failed peers stay in
    /// the file on purpose - they are tomorrow's candidates.
    #[serde(default = "default_status")]
    pub status: String,
    /// Persistent dial priority in `[MIN_PRIORITY, MAX_PRIORITY]`; higher is
    /// dialed first at startup.
    #[serde(default = "default_priority")]
    pub priority: f64,
    /// The member's `adnl.address.udp` endpoint, when registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<String>,
    /// The member's advertised `adnl.address.quic` endpoint, when registered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quic: Option<String>,
}

fn default_status() -> String {
    STATUS_UNKNOWN.to_owned()
}

impl StoredPeer {
    fn fresh(id: &PeerId, address: &str, source: &str) -> Self {
        Self {
            id: *id,
            address: address.to_owned(),
            last_seen: unix_now(),
            source: source.to_owned(),
            status: STATUS_UNKNOWN.to_owned(),
            priority: DEFAULT_PRIORITY,
            udp: None,
            quic: None,
        }
    }

    /// The address this peer would be dialed on: `address` first, then the
    /// registered UDP endpoint; `None` when neither is usable (such entries
    /// are dropped at load).
    fn dial_address(&self) -> Option<String> {
        if usable_address(&self.address) {
            return Some(self.address.clone());
        }
        let udp = self.udp.as_deref()?;
        usable_address(udp).then(|| udp.to_owned())
    }
}

/// Serde helpers encoding [`PeerId`] as 32-byte hex.
mod peer_id_hex {
    use super::{Deserialize, Deserializer, PeerId, Serializer};

    pub fn serialize<S: Serializer>(id: &PeerId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(id.as_bytes()))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PeerId, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let bytes = hex::decode(raw.trim()).map_err(serde::de::Error::custom)?;
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("peer id must be 32 bytes"))?;
        Ok(PeerId::from_bytes(array))
    }
}

/// Serialized envelope of the store file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerStoreFile {
    /// Format version; see [`PEER_STORE_VERSION`].
    pub version: u32,
    /// `mainnet` or `testnet`, matching the builder's network flag.
    pub network: String,
    /// Hex overlay id the peer set belongs to.
    pub overlay: String,
    /// Unix seconds of the save that produced the file.
    pub saved_at: u64,
    /// The peers, priority-sorted, capped at [`MAX_STORED_PEERS`].
    pub peers: Vec<StoredPeer>,
}

/// Durable peer set shared by the bootstrap dial, the growth task, the
/// background discovery task, and the save writer.
#[derive(Debug)]
pub struct PeerStore {
    path: PathBuf,
    network: String,
    overlay: String,
    /// Loaded state, corrected live by dial results and status events.
    state: tokio::sync::RwLock<HashMap<PeerId, StoredPeer>>,
    /// Peers already credited with a delivery bonus this run.
    credited: Mutex<HashSet<PeerId>>,
}

impl PeerStore {
    /// Loads `path`, applying the module-level filters; a missing, unreadable,
    /// foreign-version, or foreign-overlay file yields an empty store (the
    /// file is then rewritten for this scanner on the next save).
    pub async fn load(path: impl Into<PathBuf>, network: &str, overlay: &str) -> Arc<Self> {
        let path = path.into();
        let mut state = HashMap::new();
        match tokio::fs::read(&path).await {
            Ok(raw) => match serde_json::from_slice::<PeerStoreFile>(&raw) {
                Ok(file) if file.version != PEER_STORE_VERSION => {
                    log::warn!(
                        "peer store: ignoring {} (version {}, expected {})",
                        path.display(),
                        file.version,
                        PEER_STORE_VERSION
                    );
                }
                Ok(file) if file.network != network || file.overlay != overlay => {
                    log::warn!(
                        "peer store: ignoring {} (network={} overlay={}, expected network={} overlay={})",
                        path.display(),
                        file.network,
                        file.overlay,
                        network,
                        overlay
                    );
                }
                Ok(file) => {
                    state = filter_peers(file.peers);
                    log::info!(
                        "peer store: loaded {} peer(s) from {}",
                        state.len(),
                        path.display()
                    );
                }
                Err(error) => {
                    log::warn!("peer store: ignoring {}: {error}", path.display());
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                log::debug!("peer store: {} not present, starting cold", path.display());
            }
            Err(error) => {
                log::warn!("peer store: cannot read {}: {error}", path.display());
            }
        }
        for peer in state.values_mut() {
            // One decay pass per run: zombies sink, active peers earn it back.
            peer.priority = clamp_priority(peer.priority * RUN_DECAY);
        }
        Arc::new(Self {
            path,
            network: network.to_owned(),
            overlay: overlay.to_owned(),
            state: tokio::sync::RwLock::new(state),
            credited: Mutex::new(HashSet::new()),
        })
    }

    /// Dial candidates in priority order (descending, `last_seen` as the
    /// tie-break), each mapped to the best usable address.
    pub async fn seeds(&self) -> Vec<SeedPeer> {
        let state = self.state.read().await;
        let mut entries = state.values().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .priority
                .partial_cmp(&left.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.last_seen.cmp(&left.last_seen))
        });
        entries
            .into_iter()
            .filter_map(|entry| {
                entry.dial_address().map(|address| SeedPeer {
                    peer: entry.id,
                    address,
                })
            })
            .collect()
    }

    /// Priority used to rank `peer` among dial candidates.
    pub async fn priority(&self, peer: PeerId) -> f64 {
        self.state
            .read()
            .await
            .get(&peer)
            .map_or(DEFAULT_PRIORITY, |entry| entry.priority)
    }

    /// Reorders `peers` by persisted priority, descending; unknown peers rank
    /// at `DEFAULT_PRIORITY`. Stable, so callers keep their own order
    /// within a priority tier.
    pub async fn prioritize(&self, peers: Vec<SeedPeer>) -> Vec<SeedPeer> {
        let state = self.state.read().await;
        let rank = |peer: &PeerId| {
            state
                .get(peer)
                .map_or(DEFAULT_PRIORITY, |entry| entry.priority)
        };
        let mut peers = peers;
        peers.sort_by(|left, right| {
            rank(&right.peer)
                .partial_cmp(&rank(&left.peer))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        peers
    }

    /// Records one dial attempt.  Success adds the speed-scaled bonus and
    /// marks the peer connected; failure costs a flat penalty.  The attempt's
    /// address becomes the primary one (it is the address that was actually
    /// acted on), and a brand-new peer is inserted with `source`.
    pub async fn record_dial(
        &self,
        peer: PeerId,
        address: &str,
        source: &str,
        latency: Duration,
        ok: bool,
    ) {
        let now = unix_now();
        let mut state = self.state.write().await;
        let entry = state
            .entry(peer)
            .or_insert_with(|| StoredPeer::fresh(&peer, address, source));
        if usable_address(address) {
            address.clone_into(&mut entry.address);
        }
        if ok {
            STATUS_CONNECTED.clone_into(&mut entry.status);
            entry.last_seen = now;
            entry.priority = clamp_priority(entry.priority + DIAL_OK_BONUS * speed_factor(latency));
        } else {
            STATUS_FAILED.clone_into(&mut entry.status);
            entry.priority = clamp_priority(entry.priority - DIAL_FAIL_PENALTY);
        }
    }

    /// Applies a pool status change to the stored entry.  Peers absent from
    /// the store are not inserted: a status event carries no dialable address.
    pub async fn record_status(&self, status: &PeerStatus) {
        let now = unix_now();
        let (peer, label, penalty) = match status {
            PeerStatus::Connected { peer } => (*peer, STATUS_CONNECTED, 0.0),
            PeerStatus::Disconnected { peer } => (*peer, STATUS_DISCONNECTED, DISCONNECT_PENALTY),
            PeerStatus::Failed { peer } => (*peer, STATUS_FAILED, DISCONNECT_PENALTY),
            PeerStatus::Reconnecting { .. } => return,
        };
        let mut state = self.state.write().await;
        let Some(entry) = state.get_mut(&peer) else {
            return;
        };
        label.clone_into(&mut entry.status);
        if penalty > 0.0 {
            entry.priority = clamp_priority(entry.priority - penalty);
        } else {
            entry.last_seen = now;
        }
    }

    /// Builds the next file: merges the live candidate list, the endpoint
    /// registry, the connected set, and one delivery credit per run from the
    /// quality registry, then sorts, caps, and stamps the result.
    async fn snapshot(
        &self,
        known: &[SeedPeer],
        endpoints: &OverlayMemberEndpoints,
        connected: &[PeerId],
        quality: &PeerQualityRegistry,
    ) -> PeerStoreFile {
        let now = unix_now();
        let connected = connected.iter().copied().collect::<HashSet<_>>();
        let mut state = self.state.write().await;
        // The candidate list: everything worth redialing stays persisted,
        // including peers the current run never reached.
        for seed in known {
            let entry = state
                .entry(seed.peer)
                .or_insert_with(|| StoredPeer::fresh(&seed.peer, &seed.address, SOURCE_GROWTH));
            if usable_address(&seed.address) {
                seed.address.clone_into(&mut entry.address);
            }
        }
        // Endpoint extras: UDP/QUIC targets for probes, with an address
        // backfill for members known only from a lookup.
        if let Ok(map) = endpoints.lock() {
            for (peer, endpoint) in map.iter() {
                let entry = state
                    .entry(*peer)
                    .or_insert_with(|| StoredPeer::fresh(peer, "", SOURCE_DISCOVERY));
                // Backfill a missing or unusable primary address from the
                // registry so the entry dials even without its `udp` field.
                if !usable_address(&entry.address) {
                    entry.address = endpoint
                        .udp
                        .or(endpoint.quic)
                        .map(|address| address.to_string())
                        .unwrap_or_default();
                }
                if let Some(udp) = endpoint.udp {
                    entry.udp = Some(udp.to_string());
                }
                if let Some(quic) = endpoint.quic {
                    entry.quic = Some(quic.to_string());
                }
            }
        }
        // Live status: refresh connected peers, downgrade a stale
        // "connected" carried over from a previous run, sanitize priorities.
        for peer in state.values_mut() {
            if connected.contains(&peer.id) {
                STATUS_CONNECTED.clone_into(&mut peer.status);
                peer.last_seen = now;
            } else if peer.status == STATUS_CONNECTED {
                STATUS_DISCONNECTED.clone_into(&mut peer.status);
            }
            peer.priority = clamp_priority(peer.priority);
        }
        // One delivery credit per run: the quality registry only knows this
        // run's observations, so the bonus must not repeat on every save.
        let mut credited = self
            .credited
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for delivery in quality.snapshot() {
            let delivered =
                delivery.first_deliveries + delivery.on_time_deliveries + delivery.late_deliveries
                    > 0;
            if !(delivered && credited.insert(delivery.peer)) {
                continue;
            }
            let Some(entry) = state.get_mut(&delivery.peer) else {
                continue;
            };
            entry.priority = clamp_priority(entry.priority + DELIVERY_BONUS);
        }
        let mut peers = state.values().cloned().collect::<Vec<_>>();
        peers.sort_by(|left, right| {
            right
                .priority
                .partial_cmp(&left.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.last_seen.cmp(&left.last_seen))
        });
        peers.truncate(MAX_STORED_PEERS);
        PeerStoreFile {
            version: PEER_STORE_VERSION,
            network: self.network.clone(),
            overlay: self.overlay.clone(),
            saved_at: now,
            peers,
        }
    }

    /// Writes `file` atomically: a sibling `.tmp` file first, then a rename,
    /// so a crash mid-write can never truncate the previous good file.
    async fn persist(&self, file: &PeerStoreFile) -> Result<(), String> {
        let raw = serde_json::to_vec_pretty(file).map_err(|error| error.to_string())?;
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        tokio::fs::write(&tmp, raw)
            .await
            .map_err(|error| error.to_string())?;
        tokio::fs::rename(&tmp, &self.path)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Dedup-by-id (higher `(priority, last_seen)` wins), drop entries with no
/// usable address, priority-sort, and cap at [`MAX_STORED_PEERS`].
fn filter_peers(peers: Vec<StoredPeer>) -> HashMap<PeerId, StoredPeer> {
    let mut by_id = HashMap::new();
    for mut peer in peers {
        peer.priority = clamp_priority(peer.priority);
        if peer.dial_address().is_none() {
            log::debug!(
                "peer store: dropping {:?} - no usable dial address",
                peer.id
            );
            continue;
        }
        match by_id.entry(peer.id) {
            Entry::Vacant(vacant) => {
                vacant.insert(peer);
            }
            Entry::Occupied(mut occupied) => {
                let better = (peer.priority, peer.last_seen)
                    > (occupied.get().priority, occupied.get().last_seen);
                if better {
                    occupied.insert(peer);
                }
            }
        }
    }
    let mut entries = by_id.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .priority
            .partial_cmp(&left.priority)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.last_seen.cmp(&left.last_seen))
    });
    entries.truncate(MAX_STORED_PEERS);
    entries.into_iter().map(|peer| (peer.id, peer)).collect()
}

/// Background save loop: status events are applied live, and the file is
/// rewritten immediately (bootstrap snapshot), on every interval, and once
/// more when the manager shuts down.
pub(crate) struct PeerStoreWriter {
    pub(crate) store: Arc<PeerStore>,
    pub(crate) known: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    pub(crate) endpoints: OverlayMemberEndpoints,
    pub(crate) manager: PeerManager,
    pub(crate) quality: Arc<PeerQualityRegistry>,
    pub(crate) interval: Duration,
}

impl PeerStoreWriter {
    pub(crate) fn spawn(self) {
        let Self {
            store,
            known,
            endpoints,
            manager,
            quality,
            interval,
        } = self;
        let mut shutdown = manager.subscribe_shutdown();
        let mut statuses = manager.subscribe_statuses();
        tokio::spawn(async move {
            // The first tick fires immediately: the bootstrap snapshot lands
            // on disk before any long-lived run can be lost to a crash.
            let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(100)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        save(&store, &known, &endpoints, &manager, &quality).await;
                        return;
                    }
                    status = statuses.recv() => {
                        match status {
                            Ok(status) => store.record_status(&status).await,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                save(&store, &known, &endpoints, &manager, &quality).await;
                                return;
                            }
                            // A lagged receiver missed intermediate changes;
                            // the next save re-reads the connected set anyway.
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        }
                    }
                    _ = ticker.tick() => {
                        save(&store, &known, &endpoints, &manager, &quality).await;
                    }
                }
            }
        });
    }
}

async fn save(
    store: &Arc<PeerStore>,
    known: &Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    endpoints: &OverlayMemberEndpoints,
    manager: &PeerManager,
    quality: &Arc<PeerQualityRegistry>,
) {
    let known = known.read().await.clone();
    let connected = manager.peers().await;
    let file = store.snapshot(&known, endpoints, &connected, quality).await;
    match store.persist(&file).await {
        Ok(()) => log::debug!("peer store: saved {} peer(s)", file.peers.len()),
        Err(error) => log::warn!("peer store: save failed: {error}"),
    }
}

/// Current Unix time in seconds; `0` before the epoch.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::overlay::{PeerQualityConfig, record_member, record_member_quic};

    use super::{
        DEFAULT_PRIORITY, DELIVERY_BONUS, Duration, MAX_PRIORITY, MAX_STORED_PEERS, MIN_PRIORITY,
        OverlayMemberEndpoints, PEER_STORE_VERSION, PathBuf, PeerId, PeerQualityRegistry,
        PeerStatus, PeerStore, PeerStoreFile, RUN_DECAY, STATUS_CONNECTED, STATUS_DISCONNECTED,
        STATUS_FAILED, SeedPeer, StoredPeer, clamp_priority, speed_factor, usable_address,
    };

    fn peer(byte: u8) -> PeerId {
        PeerId::from_bytes([byte; 32])
    }

    fn stored(byte: u8, priority: f64) -> StoredPeer {
        let mut entry = StoredPeer::fresh(
            &peer(byte),
            &format!("1.2.3.4:{}", 30000 + u16::from(byte)),
            "growth",
        );
        entry.priority = priority;
        entry
    }

    /// Unique per-test path under the temp dir; tests remove their own file.
    fn path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tonutils-peer-store-{}-{tag}.json",
            std::process::id()
        ))
    }

    fn write(path: &std::path::Path, file: &PeerStoreFile) {
        std::fs::write(path, serde_json::to_vec_pretty(file).unwrap()).unwrap();
    }

    fn teardown(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    }

    #[tokio::test]
    async fn roundtrip_keeps_disconnected_peers_and_priority() {
        let path = path("roundtrip");
        teardown(&path);
        let store = PeerStore::load(&path, "mainnet", "aabb").await;
        // Fast dial: 0.5 + 0.05 * 1.0.
        store
            .record_dial(
                peer(1),
                "1.2.3.4:30001",
                "growth",
                Duration::from_millis(30),
                true,
            )
            .await;
        // Slow failing dial: 0.5 - 0.1.
        store
            .record_dial(
                peer(2),
                "1.2.3.4:30002",
                "growth",
                Duration::from_secs(2),
                false,
            )
            .await;
        // The user-visible requirement: a dropped peer stays persisted.
        store
            .record_status(&PeerStatus::Disconnected { peer: peer(1) })
            .await;
        let known = vec![SeedPeer {
            peer: peer(3),
            address: "1.2.3.4:30003".into(),
        }];
        let endpoints = OverlayMemberEndpoints::default();
        let quality = PeerQualityRegistry::new(PeerQualityConfig::default());
        let file = store.snapshot(&known, &endpoints, &[], &quality).await;
        store.persist(&file).await.unwrap();

        let reloaded = PeerStore::load(&path, "mainnet", "aabb").await;
        let state = reloaded.state.read().await;
        assert_eq!(state.len(), 3, "known peer 3 joins, no one is lost");
        let first = state.get(&peer(1)).unwrap();
        assert_eq!(first.status, STATUS_DISCONNECTED);
        assert!(
            (first.priority - 0.441).abs() < 1e-9,
            "0.45 * 0.98 decay, got {}",
            first.priority
        );
        let second = state.get(&peer(2)).unwrap();
        assert_eq!(second.status, STATUS_FAILED);
        assert!(
            (second.priority - 0.392).abs() < 1e-9,
            "0.4 * 0.98 decay, got {}",
            second.priority
        );
        drop(state);
        let seeds = reloaded.seeds().await;
        assert_eq!(seeds.len(), 3);
        assert!(
            seeds.windows(2).all(|pair| pair[0] != pair[1]),
            "candidates must not duplicate"
        );
        teardown(&path);
    }

    #[tokio::test]
    async fn load_filters_dedups_and_caps() {
        let path = path("filters");
        let mut peers = vec![
            stored(1, 0.4),
            stored(1, 0.9), // duplicate id: the higher priority wins
            stored(2, 0.5),
        ];
        peers.push(StoredPeer {
            id: peer(7),
            address: "not-an-address".into(),
            last_seen: 1,
            source: "growth".into(),
            status: STATUS_DISCONNECTED.into(),
            priority: 0.99,
            udp: None,
            quic: None,
        });
        for index in 0..MAX_STORED_PEERS + 20 {
            let mut entry = stored(10, 0.5);
            entry.id = PeerId::from_bytes({
                let mut bytes = [9u8; 32];
                bytes[..4].copy_from_slice(&u32::try_from(index).unwrap().to_be_bytes());
                bytes
            });
            entry.priority = 0.1;
            peers.push(entry);
        }
        write(
            &path,
            &PeerStoreFile {
                version: PEER_STORE_VERSION,
                network: "mainnet".into(),
                overlay: "aabb".into(),
                saved_at: 1,
                peers,
            },
        );
        let store = PeerStore::load(&path, "mainnet", "aabb").await;
        let seeds = store.seeds().await;
        assert_eq!(seeds.len(), MAX_STORED_PEERS, "capped at the limit");
        let state = store.state.read().await;
        let kept = state
            .get(&peer(1))
            .expect("the duplicate id survives with one entry");
        assert!(
            (kept.priority - 0.882).abs() < 1e-9,
            "the 0.9 entry must win the dedup, got {}",
            kept.priority
        );
        assert!(
            state.get(&peer(7)).is_none(),
            "unusable entries are dropped"
        );
        drop(state);
        assert!(
            seeds.iter().all(|seed| usable_address(&seed.address)),
            "every surviving candidate is dialable"
        );
        teardown(&path);
    }

    #[tokio::test]
    async fn foreign_version_network_or_overlay_is_ignored() {
        for (tag, version, network, overlay) in [
            ("version", PEER_STORE_VERSION + 1, "mainnet", "aabb"),
            ("network", PEER_STORE_VERSION, "testnet", "aabb"),
            ("overlay", PEER_STORE_VERSION, "mainnet", "ccdd"),
        ] {
            let path = path(tag);
            write(
                &path,
                &PeerStoreFile {
                    version,
                    network: network.into(),
                    overlay: overlay.into(),
                    saved_at: 1,
                    peers: vec![stored(1, 0.9)],
                },
            );
            let store = PeerStore::load(&path, "mainnet", "aabb").await;
            assert!(
                store.seeds().await.is_empty(),
                "{tag} mismatch must reset the store"
            );
            teardown(&path);
        }
    }

    #[tokio::test]
    async fn snapshot_marks_stale_connected_and_backfills_endpoints() {
        let path = path("snapshot");
        teardown(&path);
        let store = PeerStore::load(&path, "mainnet", "aabb").await;
        store
            .record_dial(
                peer(1),
                "1.2.3.4:30001",
                "growth",
                Duration::from_millis(10),
                true,
            )
            .await;
        assert_eq!(
            store.state.read().await.get(&peer(1)).unwrap().status,
            STATUS_CONNECTED
        );
        let endpoints = OverlayMemberEndpoints::default();
        record_member(&endpoints, peer(9), "5.6.7.8:31009".parse().unwrap());
        record_member_quic(&endpoints, peer(9), "5.6.7.8:32009".parse().unwrap());
        let quality = PeerQualityRegistry::new(PeerQualityConfig::default());
        // Connected set is empty: the stale "connected" label downgrades.
        let file = store.snapshot(&[], &endpoints, &[], &quality).await;
        assert_eq!(
            file.peers
                .iter()
                .find(|entry| entry.id == peer(1))
                .unwrap()
                .status,
            STATUS_DISCONNECTED
        );
        let backfilled = file
            .peers
            .iter()
            .find(|entry| entry.id == peer(9))
            .expect("the endpoint registry contributes members");
        assert_eq!(backfilled.address, "5.6.7.8:31009");
        assert_eq!(backfilled.udp.as_deref(), Some("5.6.7.8:31009"));
        assert_eq!(backfilled.quic.as_deref(), Some("5.6.7.8:32009"));
        teardown(&path);
    }

    #[tokio::test]
    async fn delivery_is_credited_once_per_run() {
        let path = path("delivery");
        teardown(&path);
        let store = PeerStore::load(&path, "mainnet", "aabb").await;
        store
            .record_dial(
                peer(1),
                "1.2.3.4:30001",
                "growth",
                Duration::from_millis(10),
                true,
            )
            .await;
        let quality = PeerQualityRegistry::new(PeerQualityConfig::default());
        let now = Instant::now();
        quality.set_connected(vec![peer(1)]);
        quality.record_first_delivery([7; 32], peer(1), now);
        let endpoints = OverlayMemberEndpoints::default();
        let before = store.priority(peer(1)).await;
        let first = store.snapshot(&[], &endpoints, &[peer(1)], &quality).await;
        let credited = first
            .peers
            .iter()
            .find(|entry| entry.id == peer(1))
            .unwrap()
            .priority;
        assert!(
            (credited - (before + DELIVERY_BONUS)).abs() < 1e-9,
            "a delivery adds {DELIVERY_BONUS}: {before} -> {credited}"
        );
        // A second save in the same run must not repeat the bonus.
        let again = store.snapshot(&[], &endpoints, &[peer(1)], &quality).await;
        let repeated = again
            .peers
            .iter()
            .find(|entry| entry.id == peer(1))
            .unwrap()
            .priority;
        assert!((repeated - credited).abs() < 1e-9, "one credit per run");
        teardown(&path);
    }

    #[tokio::test]
    async fn prioritize_orders_by_stored_priority() {
        let path = path("prioritize");
        teardown(&path);
        let store = PeerStore::load(&path, "mainnet", "aabb").await;
        store
            .record_dial(
                peer(1),
                "1.2.3.4:30001",
                "growth",
                Duration::from_secs(5),
                false,
            )
            .await;
        store
            .record_dial(
                peer(2),
                "1.2.3.4:30002",
                "growth",
                Duration::from_millis(5),
                true,
            )
            .await;
        let peers = vec![
            SeedPeer {
                peer: peer(1),
                address: "1.2.3.4:30001".into(),
            },
            SeedPeer {
                peer: peer(2),
                address: "1.2.3.4:30002".into(),
            },
            SeedPeer {
                peer: peer(3),
                address: "1.2.3.4:30003".into(),
            },
        ];
        let ranked = store.prioritize(peers).await;
        assert_eq!(ranked[0].peer, peer(2), "the fast, reliable peer first");
        assert_eq!(ranked[1].peer, peer(3), "unknown peers sit in the middle");
        assert_eq!(ranked[2].peer, peer(1), "the failed peer last");
        teardown(&path);
    }

    #[test]
    fn priority_arithmetic_is_clamped_and_tidy() {
        assert_eq!(speed_factor(Duration::from_millis(99)), 1.0);
        assert_eq!(speed_factor(Duration::from_millis(100)), 0.5);
        assert_eq!(speed_factor(Duration::from_secs(1)), 0.25);
        assert_eq!(clamp_priority(10.0), MAX_PRIORITY);
        assert_eq!(clamp_priority(0.0), MIN_PRIORITY);
        assert_eq!(clamp_priority(f64::NAN), DEFAULT_PRIORITY);
        assert_eq!(clamp_priority(0.555_55), 0.556);
        assert_eq!(clamp_priority(DEFAULT_PRIORITY * RUN_DECAY), 0.49);
    }
}
