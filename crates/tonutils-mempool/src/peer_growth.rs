//! Background peer-growth rounds: discovery fan-out with a managed-peer
//! budget and an event-queue idle gate.
//!
//! Split out of `builder.rs` to keep that file near the repository's 1000
//! line limit; `builder.rs` starts one task through [`PeerGrowthTask`].

use super::*;

/// Fraction of free event-queue capacity below which a growth round is
/// skipped: discovery only runs while the scanner is idle, so a congested
/// ingest path never competes with DHT lookups.
const EVENT_IDLE_FREE_RATIO: f64 = 0.25;

/// Minimum spacing between growth rounds, whichever triggers them.
///
/// A disconnect triggers a round immediately - the pool just lost a
/// member and discovery can replace it now instead of up to an interval
/// later - but a burst of disconnects must collapse into one round
/// instead of a discovery storm.
const PEER_GROWTH_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Parameters for the background peer-growth task.
pub(crate) struct PeerGrowthTask {
    pub(crate) manager: PeerManager,
    pub(crate) growth: SeedDiscoveryLookup,
    pub(crate) known: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    pub(crate) factory: OverlaySessionFactory,
    pub(crate) reconnect: ReconnectFactory,
    pub(crate) attempts: u32,
    pub(crate) backoff: Duration,
    /// Managed (non-pinned) session budget; `0` disables growth entirely.
    pub(crate) max_peers: usize,
    /// Pinned peers: excluded from `max_peers`, already registered, never
    /// re-added by growth and never evicted by quality churn.
    pub(crate) pinned: HashSet<PeerId>,
    /// Interval between periodic growth rounds.
    pub(crate) growth_interval: Duration,
    /// The scanner's event sender, read by the idle gate.
    pub(crate) event_tx: mpsc::Sender<MempoolEvent>,
    /// Durable peer store: every dial outcome is recorded so the persisted
    /// priority reflects this run's reliability and speed.
    pub(crate) store: Option<Arc<crate::peer_store::PeerStore>>,
}

impl PeerGrowthTask {
    /// Spawns the growth loop; it stops when the manager shuts down.
    pub(crate) fn spawn(self) {
        let Self {
            manager,
            growth,
            known,
            factory,
            reconnect,
            attempts,
            backoff,
            max_peers,
            pinned,
            growth_interval,
            event_tx,
            store,
        } = self;
        if max_peers == 0 {
            return;
        }
        let mut shutdown = manager.subscribe_shutdown();
        let mut statuses = manager.subscribe_statuses();
        // No round yet, so the first disconnect triggers one immediately
        // instead of waiting out the guard against bursts.
        let mut last_round: Option<tokio::time::Instant> = None;
        tokio::spawn(async move {
            // One fixed schedule for periodic rounds: a
            // disconnect-triggered round must not push the next
            // periodic tick further away, and a round that outlives
            // the interval must not turn the cadence into a tight
            // loop.
            let mut ticker = tokio::time::interval(growth_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick fires immediately; consume it so the first
            // round still happens one full interval after bootstrap.
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown.changed() => return,
                    status = statuses.recv() => {
                        let disconnect = match status {
                            Ok(status) => matches!(
                                status,
                                PeerStatus::Disconnected { .. } | PeerStatus::Failed { .. }
                            ),
                            // A lagged receiver missed status changes; keep
                            // the loop for the periodic rounds.  A closed
                            // status stream means the pool is gone.
                            Err(error) => {
                                if matches!(
                                    error,
                                    tokio::sync::broadcast::error::RecvError::Closed
                                ) {
                                    return;
                                }
                                continue;
                            }
                        };
                        if !disconnect
                            || last_round.is_some_and(|last| {
                                last.elapsed() < PEER_GROWTH_MIN_INTERVAL
                            })
                        {
                            continue;
                        }
                    }
                    _ = ticker.tick() => {}
                }
                last_round = Some(tokio::time::Instant::now());
                if !event_queue_idle(&event_tx) {
                    log::debug!("peer growth: skipped, the event queue is congested");
                    continue;
                }
                let connected = manager.peers().await;
                let managed = managed_count(&connected, &pinned);
                if managed >= max_peers {
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
                let dif = (max_peers - managed).max(1);
                let rounds = dif.min(seeds.len());
                let results = join_all((0..rounds).map(|_| growth(seeds.clone()))).await;
                let candidates = results.into_iter().flatten().collect::<Vec<_>>();
                let mut added = 0usize;
                let mut discovered = Vec::new();
                for candidate in candidates {
                    if managed_count(&manager.peers().await, &pinned) >= max_peers {
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
                    let started = Instant::now();
                    match factory(candidate.clone()).await {
                        Ok(session) => {
                            if let Some(store) = &store {
                                store
                                    .record_dial(
                                        candidate.peer,
                                        &candidate.address,
                                        crate::peer_store::SOURCE_GROWTH,
                                        started.elapsed(),
                                        true,
                                    )
                                    .await;
                            }
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
                        Err(error) => {
                            if let Some(store) = &store {
                                store
                                    .record_dial(
                                        candidate.peer,
                                        &candidate.address,
                                        crate::peer_store::SOURCE_GROWTH,
                                        started.elapsed(),
                                        false,
                                    )
                                    .await;
                            }
                            log::debug!(
                                "peer growth: session to {} failed: {error}",
                                candidate.address
                            );
                        }
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
}

/// Counts sessions that count against the growth budget: pinned sessions are
/// a separate, guaranteed set.
pub(crate) fn managed_count(connected: &[PeerId], pinned: &HashSet<PeerId>) -> usize {
    connected
        .iter()
        .filter(|peer| !pinned.contains(peer))
        .count()
}

/// Whether the scanner has enough free event-queue capacity for a discovery
/// round; below [`EVENT_IDLE_FREE_RATIO`] the round is postponed.  Shared
/// with the background peer-discovery task.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn event_queue_idle(transmitter: &mpsc::Sender<MempoolEvent>) -> bool {
    let free = transmitter.capacity() as f64;
    let total = transmitter.max_capacity() as f64;
    total <= 0.0 || free / total >= EVENT_IDLE_FREE_RATIO
}
