//! Per-peer delivery-quality tracking for overlay fan-out prioritization.
//!
//! [`MempoolScanner`](crate::MempoolScanner) records every external-message
//! delivery outcome into a [`PeerQualityRegistry`]:
//!
//! - the peer that delivers a hash first earns first-delivery credit,
//! - a duplicate delivered within [`PeerQualityConfig::late_threshold`] of the
//!   first sighting is an on-time delivery,
//! - a duplicate delivered later earns a late penalty, but only for peers that
//!   were connected at the first sighting - a peer that joined afterwards can
//!   not be judged for a message it could not have relayed earlier,
//! - a peer connected at the first sighting that delivers nothing within
//!   [`PeerQualityConfig::miss_window`] earns a miss penalty.
//!
//! Scores decay exponentially with [`PeerQualityConfig::score_halflife`] on
//! every [`PeerQualityRegistry::sweep`], so stale behavior fades and a bad
//! peer recovers once it starts delivering. Local sends
//! ([`send_external`](crate::MempoolScanner::send_external), zero `PeerId`)
//! are never attributed at all: opening a window for this node's own broadcast
//! would charge misses to peers for not echoing the message back, which
//! upstream flooding does not guarantee, so both credit and windows are
//! skipped.
//!
//! The builder spawns background tasks through [`spawn_quality_tasks`]: a
//! sweep task that refreshes the connected snapshot and closes expired
//! delivery windows, and - when quality eviction is enabled - a churn task
//! that removes the worst non-pinned peers once the managed connection count
//! exceeds `overlay_max_peers`. Sends rank sessions by score through
//! [`PeerManager::broadcast_object_in_order`].
//!
//! All recording and ranking APIs are synchronous and take explicit
//! [`Instant`] timestamps so tests stay deterministic.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::{
        Arc, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use crate::{
    mempool::MessageHash,
    overlay::{PeerId, PeerManager},
};

/// Credit for the peer that delivered a hash first.
const FIRST_CREDIT: f64 = 2.0;
/// Credit for a non-first delivery inside the late threshold.
const ON_TIME_CREDIT: f64 = 1.0;
/// Penalty for a delivery at or after the late threshold.
const LATE_PENALTY: f64 = 1.0;
/// Penalty for a connected peer that delivered nothing inside the window.
const MISS_PENALTY: f64 = 2.0;
/// Open windows never exceed this many hashes; past it a sweep runs inline
/// even when the background task is absent.
const MAX_INFLIGHT: usize = 65_536;
/// Task intervals are clamped to this so a zero-length config cannot panic
/// `tokio::time::interval`.
const MIN_TASK_INTERVAL: Duration = Duration::from_millis(1);
/// The zero `PeerId` used by the local `send_external` path; never scored.
const LOCAL_PEER: [u8; 32] = [0; 32];

/// Delivery-quality thresholds and background-task cadence.
#[derive(Clone, Debug)]
pub struct PeerQualityConfig {
    /// A duplicate at least this late after the first sighting is late.
    pub late_threshold: Duration,
    /// A peer connected at the first sighting that delivers nothing within
    /// this window earns a miss.
    pub miss_window: Duration,
    /// [`PeerQualityRegistry::eviction_candidates`] never judges a peer with
    /// fewer observations, so noisy scores on thin evidence cannot evict a
    /// peer that simply has not been observed long enough.
    pub min_observations: u64,
    /// Connected-snapshot refresh and window-sweep cadence.
    pub sweep_interval: Duration,
    /// Eviction-pass cadence.
    pub churn_interval: Duration,
    /// Every score halves over this much swept time.
    pub score_halflife: Duration,
    /// Stats for a peer that is neither connected nor observed for this long
    /// are dropped on the next sweep.
    pub stats_retention: Duration,
}

impl Default for PeerQualityConfig {
    fn default() -> Self {
        Self {
            late_threshold: Duration::from_millis(500),
            miss_window: Duration::from_secs(3),
            min_observations: 10,
            sweep_interval: Duration::from_secs(1),
            churn_interval: Duration::from_secs(60),
            score_halflife: Duration::from_secs(60),
            stats_retention: Duration::from_secs(600),
        }
    }
}

/// Point-in-time view of one peer's delivery statistics.
#[derive(Clone, Debug)]
pub struct PeerQualitySnapshot {
    /// The observed peer.
    pub peer: PeerId,
    /// Current decayed score; higher is better.
    pub score: f64,
    /// Total recorded delivery outcomes.
    pub observations: u64,
    /// Deliveries that arrived first.
    pub first_deliveries: u64,
    /// Non-first deliveries inside the late threshold.
    pub on_time_deliveries: u64,
    /// Deliveries at or after the late threshold.
    pub late_deliveries: u64,
    /// Missed delivery windows.
    pub misses: u64,
    /// When this peer last delivered anything.
    pub last_delivery: Option<Instant>,
}

/// Accumulated delivery statistics for one peer.
#[derive(Clone, Debug, Default)]
struct PeerStats {
    first: u64,
    on_time: u64,
    late: u64,
    misses: u64,
    score: f64,
    last_delivery: Option<Instant>,
    last_activity: Option<Instant>,
}

impl PeerStats {
    fn observations(&self) -> u64 {
        self.first + self.on_time + self.late + self.misses
    }
}

/// An open delivery window for one hash.
#[derive(Debug)]
struct Inflight {
    first_seen: Instant,
    /// Peers connected when the hash was first seen; only these are judged
    /// late or missing. Shared across windows opened inside one snapshot.
    connected: Arc<Vec<PeerId>>,
    delivered: HashSet<PeerId>,
}

/// Mutable registry state behind the lock.
#[derive(Debug)]
struct RegistryState {
    stats: HashMap<PeerId, PeerStats>,
    inflight: HashMap<MessageHash, Inflight>,
    connected: Arc<Vec<PeerId>>,
    last_sweep: Instant,
}

/// Tracks per-peer delivery outcomes, decays scores, and ranks peers.
///
/// See the [module documentation](self) for the scoring model.
#[derive(Debug)]
pub struct PeerQualityRegistry {
    config: PeerQualityConfig,
    state: RwLock<RegistryState>,
    /// Peers removed by quality churn since this registry was created,
    /// surfaced by the live census so churn is measurable per run.
    evictions: AtomicU64,
}

impl PeerQualityRegistry {
    /// Creates an empty registry with the given thresholds.
    pub fn new(config: PeerQualityConfig) -> Self {
        Self {
            config,
            state: RwLock::new(RegistryState {
                stats: HashMap::new(),
                inflight: HashMap::new(),
                connected: Arc::new(Vec::new()),
                last_sweep: Instant::now(),
            }),
            evictions: AtomicU64::new(0),
        }
    }

    /// Records one peer removed by quality churn.
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, AtomicOrdering::Relaxed);
    }

    /// Total peers evicted by quality churn since this registry was created.
    #[must_use]
    pub fn evictions(&self) -> u64 {
        self.evictions.load(AtomicOrdering::Relaxed)
    }

    /// Returns the immutable configuration this registry was built with.
    pub fn config(&self) -> &PeerQualityConfig {
        &self.config
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, RegistryState> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, RegistryState> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the connected-peer snapshot used for miss attribution.
    ///
    /// The background sweep task refreshes it every
    /// [`PeerQualityConfig::sweep_interval`], so the snapshot can be stale by
    /// up to one interval: a peer that connected after that is missing from
    /// windows opened meanwhile and is not charged, and a peer that left
    /// recently may still be charged for one last window.
    pub fn set_connected(&self, peers: Vec<PeerId>) {
        self.write_state().connected = Arc::new(peers);
    }

    /// Records a first-seen delivery: credits the peer and opens the delivery
    /// window that later closes misses for every other connected peer.
    ///
    /// The zero `PeerId` (local `send_external`) is ignored entirely.
    pub fn record_first_delivery(&self, hash: MessageHash, peer: PeerId, now: Instant) {
        if peer.as_bytes() == LOCAL_PEER {
            return;
        }
        let mut state = self.write_state();
        if self.sweep_due(&state, now) {
            self.sweep_locked(&mut state, now);
        }
        let connected = Arc::clone(&state.connected);
        let stats = state.stats.entry(peer).or_default();
        stats.first += 1;
        stats.score += FIRST_CREDIT;
        stats.last_delivery = Some(now);
        stats.last_activity = Some(now);
        state.inflight.entry(hash).or_insert_with(|| Inflight {
            first_seen: now,
            connected,
            delivered: HashSet::from([peer]),
        });
    }

    /// Records a duplicate delivery of an open window: an on-time delivery
    /// inside the late threshold, a late penalty afterwards.
    ///
    /// Duplicates without an open window - a local send, a window already
    /// closed by a miss sweep, or a hash whose first delivery came from this
    /// node - carry no timing evidence and are ignored.
    pub fn record_duplicate_delivery(&self, hash: MessageHash, peer: PeerId, now: Instant) {
        if peer.as_bytes() == LOCAL_PEER {
            return;
        }
        let mut state = self.write_state();
        let Some(record) = state.inflight.get_mut(&hash) else {
            return;
        };
        if !record.delivered.insert(peer) {
            return;
        }
        let first_seen = record.first_seen;
        let judged = record.connected.contains(&peer);
        let lateness = now.saturating_duration_since(first_seen);
        let stats = state.stats.entry(peer).or_default();
        stats.last_delivery = Some(now);
        stats.last_activity = Some(now);
        if judged && lateness >= self.config.late_threshold {
            stats.late += 1;
            stats.score -= LATE_PENALTY;
        } else {
            stats.on_time += 1;
            stats.score += ON_TIME_CREDIT;
        }
    }

    /// Closes every window whose miss window elapsed, charging misses to the
    /// peers connected at first sighting that never delivered, then decays all
    /// scores and prunes stale stats.
    pub fn sweep(&self, now: Instant) {
        let mut state = self.write_state();
        self.sweep_locked(&mut state, now);
    }

    /// Returns the connected peers sorted best score first for fan-out
    /// ordering. Peers without stats (score `0.0`) sort between positive and
    /// negative peers; ties break on the peer id for determinism.
    pub fn rank(&self, peers: &[PeerId]) -> Vec<PeerId> {
        let state = self.read_state();
        let score = |peer: &PeerId| state.stats.get(peer).map_or(0.0, |stats| stats.score);
        let mut ranked = peers.to_vec();
        ranked.sort_by(|left, right| {
            score(right)
                .partial_cmp(&score(left))
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.as_bytes().cmp(&right.as_bytes()))
        });
        ranked
    }

    /// Returns up to `connected.len().saturating_sub(max_managed)` non-pinned
    /// peers to evict so that at most `max_managed` managed sessions remain:
    /// the worst-scoring observed peers first, protected from eviction until
    /// they reach [`PeerQualityConfig::min_observations`].
    ///
    /// Fewer candidates than needed are returned when too few peers are
    /// observed; the caller then stays over the limit rather than evicting a
    /// peer on thin evidence.
    pub fn eviction_candidates(
        &self,
        connected: &[PeerId],
        pinned: &HashSet<PeerId>,
        max_managed: usize,
    ) -> Vec<PeerId> {
        let state = self.read_state();
        let managed = connected
            .iter()
            .copied()
            .filter(|peer| !pinned.contains(peer))
            .collect::<Vec<_>>();
        if managed.len() <= max_managed {
            return Vec::new();
        }
        let excess = managed.len() - max_managed;
        let min_observations = self.config.min_observations;
        let mut candidates = managed
            .iter()
            .filter_map(|peer| {
                let stats = state.stats.get(peer)?;
                (stats.observations() >= min_observations).then_some((
                    stats.score,
                    stats.last_delivery,
                    *peer,
                ))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| match (left.1, right.1) {
                    (Some(left), Some(right)) => left.cmp(&right),
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                })
                .then_with(|| left.2.as_bytes().cmp(&right.2.as_bytes()))
        });
        candidates
            .into_iter()
            .take(excess)
            .map(|(_, _, peer)| peer)
            .collect()
    }

    /// Returns a score-sorted snapshot of every tracked peer for logging.
    pub fn snapshot(&self) -> Vec<PeerQualitySnapshot> {
        let state = self.read_state();
        let mut snapshot = state
            .stats
            .iter()
            .map(|(peer, stats)| PeerQualitySnapshot {
                peer: *peer,
                score: stats.score,
                observations: stats.observations(),
                first_deliveries: stats.first,
                on_time_deliveries: stats.on_time,
                late_deliveries: stats.late,
                misses: stats.misses,
                last_delivery: stats.last_delivery,
            })
            .collect::<Vec<_>>();
        snapshot.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.peer.as_bytes().cmp(&right.peer.as_bytes()))
        });
        snapshot
    }

    /// Whether a full sweep is due, or forced by an unbounded window map when
    /// no background task runs.
    fn sweep_due(&self, state: &RegistryState, now: Instant) -> bool {
        now.saturating_duration_since(state.last_sweep) >= self.config.sweep_interval
            || state.inflight.len() >= MAX_INFLIGHT
    }

    fn sweep_locked(&self, state: &mut RegistryState, now: Instant) {
        let window = self.config.miss_window;
        let expired = state
            .inflight
            .iter()
            .filter(|(_, record)| now.saturating_duration_since(record.first_seen) >= window)
            .map(|(hash, _)| *hash)
            .collect::<Vec<_>>();
        for hash in expired {
            let Some(record) = state.inflight.remove(&hash) else {
                continue;
            };
            for peer in record.connected.iter() {
                if record.delivered.contains(peer) {
                    continue;
                }
                let stats = state.stats.entry(*peer).or_default();
                stats.misses += 1;
                stats.score -= MISS_PENALTY;
                stats.last_activity = Some(now);
            }
        }
        let elapsed = now.saturating_duration_since(state.last_sweep);
        if !elapsed.is_zero() && !self.config.score_halflife.is_zero() {
            let factor =
                0.5f64.powf(elapsed.as_secs_f64() / self.config.score_halflife.as_secs_f64());
            for stats in state.stats.values_mut() {
                stats.score *= factor;
            }
        }
        state.last_sweep = now;
        let retention = self.config.stats_retention;
        let connected = state.connected.iter().copied().collect::<HashSet<_>>();
        state.stats.retain(|peer, stats| {
            connected.contains(peer)
                || stats
                    .last_activity
                    .is_some_and(|at| now.saturating_duration_since(at) < retention)
        });
    }
}

/// Spawns the background quality tasks: a sweep task that refreshes the
/// connected snapshot and closes delivery windows every
/// [`PeerQualityConfig::sweep_interval`], and - when `max_managed` is set -
/// an eviction task that removes the worst non-pinned peers every
/// [`PeerQualityConfig::churn_interval`].
///
/// Both tasks stop when the manager shuts down. `max_managed` is the
/// `overlay_max_peers` cap for non-pinned sessions; `0` therefore evicts
/// every managed peer.
pub(crate) fn spawn_quality_tasks(
    registry: Arc<PeerQualityRegistry>,
    manager: PeerManager,
    pinned: HashSet<PeerId>,
    max_managed: Option<usize>,
) {
    let sweep_interval = registry.config().sweep_interval.max(MIN_TASK_INTERVAL);
    let sweep_registry = Arc::clone(&registry);
    let sweep_manager = manager.clone();
    let mut sweep_shutdown = sweep_manager.subscribe_shutdown();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(sweep_interval);
        loop {
            tokio::select! {
                _ = sweep_shutdown.changed() => return,
                _ = tick.tick() => {}
            }
            sweep_registry.set_connected(sweep_manager.peers().await);
            sweep_registry.sweep(Instant::now());
        }
    });

    let Some(max_managed) = max_managed else {
        return;
    };
    let churn_interval = registry.config().churn_interval.max(MIN_TASK_INTERVAL);
    let mut churn_shutdown = manager.subscribe_shutdown();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(churn_interval);
        loop {
            tokio::select! {
                _ = churn_shutdown.changed() => return,
                _ = tick.tick() => {}
            }
            let connected = manager.peers().await;
            for peer in registry.eviction_candidates(&connected, &pinned, max_managed) {
                log::info!(
                    "peer quality: evicting {peer:?}, the worst delivery score over the limit"
                );
                manager.remove_session(peer).await;
                registry.record_eviction();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(byte: u8) -> PeerId {
        PeerId::from_bytes([byte; 32])
    }

    fn scores(registry: &PeerQualityRegistry) -> HashMap<PeerId, PeerQualitySnapshot> {
        registry
            .snapshot()
            .into_iter()
            .map(|snapshot| (snapshot.peer, snapshot))
            .collect()
    }

    #[test]
    fn first_delivery_credits_the_first_peer_and_ranks_it() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig::default());
        let now = Instant::now();
        registry.set_connected(vec![peer(1), peer(2)]);
        registry.record_first_delivery([9; 32], peer(1), now);
        let ranked = registry.rank(&[peer(1), peer(2)]);
        assert_eq!(ranked, vec![peer(1), peer(2)]);
        let snapshot = scores(&registry);
        assert_eq!(snapshot[&peer(1)].first_deliveries, 1);
        assert_eq!(snapshot[&peer(1)].observations, 1);
        assert!(!snapshot.contains_key(&peer(2)));
    }

    #[test]
    fn duplicates_are_scored_by_the_late_threshold() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig::default());
        let now = Instant::now();
        registry.set_connected(vec![peer(1), peer(2), peer(3)]);
        registry.record_first_delivery([1; 32], peer(1), now);
        registry.record_duplicate_delivery([1; 32], peer(2), now + Duration::from_millis(100));
        registry.record_duplicate_delivery([1; 32], peer(3), now + Duration::from_millis(600));
        let snapshot = scores(&registry);
        assert_eq!(snapshot[&peer(2)].on_time_deliveries, 1);
        assert_eq!(snapshot[&peer(3)].late_deliveries, 1);
        assert!(
            snapshot[&peer(1)].score > snapshot[&peer(2)].score,
            "the first peer must rank above an on-time duplicate"
        );
        assert!(
            snapshot[&peer(2)].score > snapshot[&peer(3)].score,
            "an on-time duplicate must rank above a late delivery"
        );
    }

    #[test]
    fn late_judgement_only_applies_to_peers_connected_at_first_sighting() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig::default());
        let now = Instant::now();
        registry.set_connected(vec![peer(1), peer(2)]);
        registry.record_first_delivery([2; 32], peer(1), now);
        registry.set_connected(vec![peer(1), peer(2), peer(3)]);
        // peer(3) joined after the first sighting and delivers well past the
        // late threshold: credited, never penalized.
        registry.record_duplicate_delivery([2; 32], peer(3), now + Duration::from_millis(600));
        registry.sweep(now + Duration::from_secs(4));
        let snapshot = scores(&registry);
        assert_eq!(snapshot[&peer(1)].misses, 0);
        assert_eq!(snapshot[&peer(2)].misses, 1);
        assert_eq!(snapshot[&peer(3)].misses, 0);
        assert_eq!(snapshot[&peer(3)].on_time_deliveries, 1);
        assert!(
            snapshot[&peer(2)].score < 0.0,
            "a silent connected peer must be charged for the missed window"
        );
    }

    #[test]
    fn local_origin_sends_are_never_attributed() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig::default());
        let now = Instant::now();
        registry.set_connected(vec![peer(1)]);
        registry.record_first_delivery([3; 32], PeerId::from_bytes(LOCAL_PEER), now);
        registry.record_duplicate_delivery([3; 32], peer(1), now + Duration::from_millis(10));
        assert!(
            registry.snapshot().is_empty(),
            "local sends must open no window and credit no peer"
        );
    }

    #[test]
    fn sweep_decays_scores_over_the_half_life() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig {
            score_halflife: Duration::from_secs(60),
            ..PeerQualityConfig::default()
        });
        let now = Instant::now();
        registry.set_connected(vec![peer(1)]);
        registry.record_first_delivery([4; 32], peer(1), now);
        registry.sweep(now + Duration::from_secs(60));
        let score = registry.snapshot()[0].score;
        assert!(
            (score - 1.0).abs() < 0.01,
            "score {score} should halve from {FIRST_CREDIT} over one half-life"
        );
    }

    #[test]
    fn eviction_keeps_pinned_and_under_observed_peers() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig {
            min_observations: 2,
            ..PeerQualityConfig::default()
        });
        let pinned = peer(9);
        let silent = peer(8);
        let good = peer(7);
        let now = Instant::now();
        registry.set_connected(vec![pinned, silent, good]);
        for (index, offset) in [(0u8, 0u64), (1, 5)] {
            let first_seen = now + Duration::from_secs(offset);
            registry.record_first_delivery([index; 32], good, first_seen);
            registry.sweep(first_seen + Duration::from_secs(4));
        }
        // A peer that joins after both windows has no evidence yet.
        let fresh = peer(6);
        registry.set_connected(vec![pinned, silent, good, fresh]);
        let snapshot = scores(&registry);
        assert_eq!(snapshot[&good].first_deliveries, 2);
        assert_eq!(snapshot[&silent].misses, 2);

        let connected = [pinned, silent, good, fresh];
        let single = registry.eviction_candidates(&connected, &HashSet::from([pinned]), 2);
        assert_eq!(single, vec![silent], "the worst observed peer goes first");
        let all_observed = registry.eviction_candidates(&connected, &HashSet::from([pinned]), 0);
        assert_eq!(
            all_observed,
            vec![silent, good],
            "both observed peers are evictable, worst first"
        );
        assert!(
            !all_observed.contains(&pinned) && !all_observed.contains(&fresh),
            "pinned and under-observed peers must be protected"
        );
        assert_eq!(
            registry.eviction_candidates(&connected, &HashSet::from([pinned]), 5),
            [],
            "at or under the limit no candidate exists"
        );
    }

    #[test]
    fn sweep_prunes_disconnected_stats_after_retention() {
        let registry = PeerQualityRegistry::new(PeerQualityConfig {
            stats_retention: Duration::from_secs(60),
            ..PeerQualityConfig::default()
        });
        let now = Instant::now();
        registry.set_connected(vec![peer(1)]);
        registry.record_first_delivery([5; 32], peer(1), now);
        registry.set_connected(Vec::new());
        registry.sweep(now + Duration::from_secs(3600));
        assert!(registry.snapshot().is_empty());
    }
}
