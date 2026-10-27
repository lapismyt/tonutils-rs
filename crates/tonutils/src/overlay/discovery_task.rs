//! Background overlay discovery rounds that run while the scanner is idle.
//!
//! With a warm [`crate::overlay::PeerStore`] the blocking start-time lookup is skipped,
//! and [`PeerDiscoveryTask`] replaces it: every interval it re-runs the same
//! configured lookup the builder would have run at startup (DHT
//! `overlay.<id>.nodes` via the installed
//! [`DiscoveryLookup`](tonutils::overlay::DiscoveryLookup) /
//! [`TypedDiscoveryLookup`](tonutils::overlay::TypedDiscoveryLookup) /
//! [`SeedDiscoveryLookup`]), folds fresh members into the known-candidate
//! list, and dials - in persisted-priority order, up to the managed budget -
//! both brand-new members and known peers whose session is currently down.
//!
//! Split out of `builder.rs` to keep that file near the repository's 1000
//! line limit.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    overlay::{
        DiscoveryConfig, DiscoveryLookup, OverlayId, PeerId, PeerManager, ReconnectFactory,
        SeedDiscoveryLookup, SeedPeer, TypedDiscoveryLookup,
    },
    overlay::{
        OverlayPeerPool, OverlaySessionFactory,
        peer_growth::{event_queue_idle, managed_count},
        peer_store::{PeerStore, SOURCE_DISCOVERY},
    },
};

use futures::future::BoxFuture;
/// Default spacing between background discovery rounds.  Rounds also require
/// an idle event queue, so the lookup never competes with ingest.
pub(crate) const DEFAULT_PEER_STORE_DISCOVERY_INTERVAL: Duration = Duration::from_secs(60);

/// One discovery lookup, normalized across the three builder variants:
/// bootstrap seeds in, dialable overlay members out, with the configured
/// timeout and record cap applied inside.
pub(crate) type DiscoveryRunner =
    Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, Vec<SeedPeer>> + Send + Sync>;

/// Normalizes the configured lookup variants into a [`DiscoveryRunner`],
/// preserving the startup semantics of each: the seed lookup runs directly
/// under the timeout, while the record and typed lookups go through
/// [`DiscoveryConfig`]'s deterministic seed fallback.  `None` when no lookup
/// is configured.
pub(crate) fn discovery_runner(
    seed_lookup: Option<SeedDiscoveryLookup>,
    typed_lookup: Option<TypedDiscoveryLookup>,
    record_lookup: Option<DiscoveryLookup>,
    overlay: OverlayId,
    timeout: Duration,
) -> Option<DiscoveryRunner> {
    if let Some(lookup) = seed_lookup {
        return Some(Arc::new(move |seeds| {
            let lookup = Arc::clone(&lookup);
            Box::pin(async move {
                tokio::time::timeout(timeout, lookup(seeds))
                    .await
                    .ok()
                    .filter(|peers| !peers.is_empty())
                    .unwrap_or_default()
            })
        }));
    }
    let base = DiscoveryConfig {
        overlay,
        seeds: Vec::new(),
        lookup_timeout: timeout,
        max_records: 64,
    };
    if let Some(lookup) = typed_lookup {
        let base = base.clone();
        return Some(Arc::new(move |seeds| {
            let lookup = Arc::clone(&lookup);
            let mut config = base.clone();
            config.seeds = seeds;
            Box::pin(async move { config.discover_typed(move |seeds| lookup(seeds)).await })
        }));
    }
    if let Some(lookup) = record_lookup {
        let base = base.clone();
        return Some(Arc::new(move |seeds| {
            let lookup = Arc::clone(&lookup);
            let mut config = base.clone();
            config.seeds = seeds;
            Box::pin(async move { config.discover_with(move |seeds| lookup(seeds)).await })
        }));
    }
    None
}

/// Parameters for the background discovery task.
pub(crate) struct PeerDiscoveryTask {
    pub(crate) manager: PeerManager,
    pub(crate) runner: DiscoveryRunner,
    /// Bootstrap DHT seeds: every round asks them for `overlay.<id>.nodes`
    /// (transport lookups additionally prefer the endpoint registry).
    pub(crate) seeds: Vec<SeedPeer>,
    pub(crate) known: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    pub(crate) factory: OverlaySessionFactory,
    pub(crate) reconnect: ReconnectFactory,
    pub(crate) attempts: u32,
    pub(crate) backoff: Duration,
    /// Managed (non-pinned) session budget; `0` disables the task.
    pub(crate) max_peers: usize,
    /// Pinned sessions are never re-dialed or re-added here.
    pub(crate) pinned: HashSet<PeerId>,
    pub(crate) interval: Duration,
    /// Packet queue read by the idle gate.
    pub(crate) pool: Arc<OverlayPeerPool>,
    pub(crate) store: Option<Arc<PeerStore>>,
}

impl PeerDiscoveryTask {
    /// Spawns the round loop; it stops when the manager shuts down.
    ///
    /// The first round happens one full interval after bootstrap (the
    /// immediate first tick is consumed), mirroring the growth task so a warm
    /// startup is not followed by an instant lookup burst.
    #[expect(
        clippy::too_many_lines,
        reason = "The scan loop keeps discovery, prioritization, dialing, and persistence in one auditable sequence"
    )]
    pub(crate) fn spawn(self) {
        let Self {
            manager,
            runner,
            seeds,
            known,
            factory,
            reconnect,
            attempts,
            backoff,
            max_peers,
            pinned,
            interval,
            pool,
            store,
        } = self;
        if max_peers == 0 {
            return;
        }
        let mut shutdown = manager.subscribe_shutdown();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(100)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown.changed() => return,
                    _ = ticker.tick() => {}
                }
                // Idle moments only: a congested ingest queue postpones the
                // round instead of competing with it.
                if !event_queue_idle(&pool) {
                    log::debug!("peer discovery: skipped, the event queue is congested");
                    continue;
                }
                let found = runner(seeds.clone()).await;
                let merged = merge_found(&known, &pinned, found).await;
                if merged > 0 {
                    log::debug!("peer discovery: merged {merged} new or refreshed member(s)");
                }
                // Dial candidates: everything known whose session is down,
                // best persistent priority first, bounded by the budget.
                let connected = manager.peers().await.into_iter().collect::<HashSet<_>>();
                let mut queue = known
                    .read()
                    .await
                    .iter()
                    .filter(|seed| !connected.contains(&seed.peer) && !pinned.contains(&seed.peer))
                    .cloned()
                    .collect::<Vec<_>>();
                if let Some(store) = &store {
                    queue = store.prioritize(queue).await;
                }
                let mut added = 0usize;
                for candidate in queue {
                    if managed_count(&manager.peers().await, &pinned) >= max_peers {
                        break;
                    }
                    if manager.peers().await.contains(&candidate.peer) {
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
                                        SOURCE_DISCOVERY,
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
                            added += 1;
                        }
                        Err(error) => {
                            if let Some(store) = &store {
                                store
                                    .record_dial(
                                        candidate.peer,
                                        &candidate.address,
                                        SOURCE_DISCOVERY,
                                        started.elapsed(),
                                        false,
                                    )
                                    .await;
                            }
                            log::debug!(
                                "peer discovery: session to {} failed: {error}",
                                candidate.address
                            );
                        }
                    }
                }
                if added > 0 {
                    log::info!(
                        "peer discovery: added {added} session(s), registered_peers={}",
                        manager.peer_count().await
                    );
                }
            }
        });
    }
}

/// Folds discovery results into the known list.  New members are appended; an
/// existing member keeps its slot but takes the fresher signed address, which
/// then also becomes the redial target.  Returns how many entries changed.
async fn merge_found(
    known: &Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    pinned: &HashSet<PeerId>,
    found: Vec<SeedPeer>,
) -> usize {
    let mut merged = 0usize;
    let mut known = known.write().await;
    for peer in found {
        if !peer.is_valid() || pinned.contains(&peer.peer) {
            continue;
        }
        match known.iter_mut().find(|seed| seed.peer == peer.peer) {
            None => {
                known.push(peer);
                merged += 1;
            }
            Some(existing) if existing.address != peer.address => {
                log::debug!(
                    "peer discovery: refreshing {peer:?}: {} -> {}",
                    existing.address,
                    peer.address
                );
                existing.address = peer.address;
                merged += 1;
            }
            Some(_) => {}
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use crate::overlay::{OverlayConfig, OverlaySession};
    use futures::future::BoxFuture;

    use super::{
        Arc, DiscoveryRunner, Duration, HashSet, OverlayId, OverlaySessionFactory,
        PeerDiscoveryTask, PeerId, PeerManager, PeerStore, ReconnectFactory, SeedDiscoveryLookup,
        SeedPeer, discovery_runner, merge_found,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct PendingSession {
        peer: PeerId,
    }

    impl OverlaySession for PendingSession {
        fn peer_id(&self) -> PeerId {
            self.peer
        }

        fn receive(&mut self) -> BoxFuture<'_, Result<Arc<[u8]>, String>> {
            Box::pin(std::future::pending())
        }

        fn send(&mut self, _payload: Arc<[u8]>) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn peer(byte: u8) -> PeerId {
        PeerId::from_bytes([byte; 32])
    }

    fn seed(byte: u8) -> SeedPeer {
        SeedPeer {
            peer: peer(byte),
            address: format!("1.2.3.4:{}", 31000 + u16::from(byte)),
        }
    }

    fn store_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tonutils-peer-discovery-{}-{tag}.json",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn runner_normalizes_the_seed_lookup_with_timeout_and_empty_filter() {
        let calls = Arc::new(AtomicUsize::new(0));
        let lookup: SeedDiscoveryLookup = {
            let calls = Arc::clone(&calls);
            Arc::new(move |seeds| {
                calls.fetch_add(1, Ordering::SeqCst);
                let seeds = seeds.clone();
                Box::pin(async move { seeds })
            })
        };
        let runner = discovery_runner(
            Some(lookup),
            None,
            None,
            OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
            Duration::from_secs(1),
        )
        .expect("the seed lookup must produce a runner");
        let result = runner(vec![seed(1)]).await;
        assert_eq!(result.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // An empty answer is filtered out; the caller falls back to seeds.
        let empty: SeedDiscoveryLookup = Arc::new(|_| Box::pin(async { Vec::new() }));
        let runner = discovery_runner(
            Some(empty),
            None,
            None,
            OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            runner(vec![seed(2)]).await,
            [] as [SeedPeer; 0],
            "empty answers are filtered so the caller can fall back to seeds"
        );
    }

    #[test]
    fn runner_is_none_without_a_lookup() {
        assert!(
            discovery_runner(
                None,
                None,
                None,
                OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
                Duration::from_secs(1),
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn rounds_merge_new_members_and_dial_them_by_priority() {
        let path = store_path("rounds");
        let _ = std::fs::remove_file(&path);
        let manager = PeerManager::with_overlay(
            OverlayConfig::default(),
            OverlayId::MAINNET_BASECHAIN_OVERLAY_ID,
        )
        .expect("manager");
        let found = seed(42);
        let calls = Arc::new(AtomicUsize::new(0));
        let runner: DiscoveryRunner = {
            let calls = Arc::clone(&calls);
            let found = found.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                let found = found.clone();
                Box::pin(async move { vec![found] })
            })
        };
        // The known list holds a failed peer (low priority) and a fresh one.
        let known = Arc::new(tokio::sync::RwLock::new(vec![seed(1)]));
        let store = PeerStore::load(&path, "mainnet", "aa").await;
        store
            .record_dial(
                peer(1),
                "1.2.3.4:31001",
                "growth",
                Duration::from_secs(3),
                false,
            )
            .await;
        let factory: OverlaySessionFactory = Arc::new(move |candidate| {
            Box::pin(async move {
                Ok(Box::new(PendingSession {
                    peer: candidate.peer,
                }) as Box<dyn OverlaySession>)
            })
        });
        let reconnect: ReconnectFactory = Arc::new(move |peer| {
            Box::pin(async move {
                Err::<Box<dyn OverlaySession>, String>(format!("no reconnect for {peer:?}"))
            })
        });
        let pool = manager.pool();
        PeerDiscoveryTask {
            manager: manager.clone(),
            runner,
            seeds: vec![seed(9)],
            known: Arc::clone(&known),
            factory,
            reconnect,
            attempts: 0,
            backoff: Duration::ZERO,
            max_peers: 8,
            pinned: HashSet::new(),
            interval: Duration::from_millis(50),
            pool,
            store: Some(Arc::clone(&store)),
        }
        .spawn();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(calls.load(Ordering::SeqCst) >= 1, "the round ran");
        assert!(
            known
                .read()
                .await
                .iter()
                .any(|entry| entry.peer == peer(42)),
            "the discovered member joins the known list"
        );
        assert!(
            manager.peers().await.contains(&peer(42)),
            "the discovered member is dialed"
        );
        assert!(
            manager.peers().await.contains(&peer(1)),
            "known-but-down peers are redialed too"
        );
        assert!(
            store
                .seeds()
                .await
                .iter()
                .any(|entry| entry.peer == peer(42)),
            "the new member persists into the store"
        );
        manager.shutdown();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn merge_prefers_the_fresher_address_and_skips_pinned() {
        let known = Arc::new(tokio::sync::RwLock::new(vec![seed(1)]));
        let pinned = HashSet::from([peer(2)]);
        let merged = merge_found(
            &known,
            &pinned,
            vec![
                SeedPeer {
                    peer: peer(1),
                    address: "9.9.9.9:1111".into(),
                },
                seed(2),
                seed(3),
                SeedPeer {
                    peer: peer(4),
                    address: "not-an-address".into(),
                },
            ],
        )
        .await;
        assert_eq!(merged, 2, "the refreshed address and the new member");
        let known = known.read().await;
        assert_eq!(known.len(), 2, "pinned and invalid members stay out");
        assert_eq!(known[0].address, "9.9.9.9:1111", "fresher address wins");
        assert_eq!(known[1].peer, peer(3));
    }
}
