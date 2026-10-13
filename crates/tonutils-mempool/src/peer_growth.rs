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
    /// The scanner's event sender, read by the idle gate.
    pub(crate) event_tx: mpsc::Sender<MempoolEvent>,
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
            event_tx,
        } = self;
        if max_peers == 0 {
            return;
        }
        let mut shutdown = manager.subscribe_shutdown();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => return,
                    () = tokio::time::sleep(PEER_GROWTH_INTERVAL) => {}
                }
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
}

/// Counts sessions that count against the growth budget: pinned sessions are
/// a separate, guaranteed set.
fn managed_count(connected: &[PeerId], pinned: &HashSet<PeerId>) -> usize {
    connected
        .iter()
        .filter(|peer| !pinned.contains(peer))
        .count()
}

/// Whether the scanner has enough free event-queue capacity for a discovery
/// round; below [`EVENT_IDLE_FREE_RATIO`] the round is postponed.
#[allow(clippy::cast_precision_loss)]
fn event_queue_idle(transmitter: &mpsc::Sender<MempoolEvent>) -> bool {
    let free = transmitter.capacity() as f64;
    let total = transmitter.max_capacity() as f64;
    total <= 0.0 || free / total >= EVENT_IDLE_FREE_RATIO
}
