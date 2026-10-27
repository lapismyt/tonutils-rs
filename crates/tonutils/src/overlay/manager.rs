//! `PeerManager`: session registry, receive and reconnect tasks, and
//! broadcast fan-out.
//!
//! Split out of `lib.rs` to keep that file within the repository's 1000
//! line limit; the public type and the supporting structs stay in
//! `lib.rs` and only the inherent impls moved here.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::overlay::{
    INGEST_WAIT_TIMEOUT, OverlayConfig, OverlayError, OverlayId, OverlayPacket, OverlayPeerPool,
    OverlaySession, OverlaySessionSender, PeerId, PeerManager, PeerStatus, ReconnectFactory,
    RoutingMetadata,
};

use tokio::sync::{Mutex, RwLock, broadcast, oneshot, watch};

/// One registered session: the mutex the receive loop drives `receive()`
/// through, plus the session's lock-free send handle when it has one.
///
/// Sends prefer [`SharedSession::sender`], because the receive loop
/// holds the mutex while `receive()` waits for the next inbound
/// payload; only sessions without a handle fall back to the mutex.
#[derive(Clone)]
pub(crate) struct SharedSession {
    session: Arc<Mutex<Box<dyn OverlaySession>>>,
    sender: Option<Arc<dyn OverlaySessionSender>>,
}

impl SharedSession {
    fn wrap(session: Box<dyn OverlaySession>) -> Self {
        let sender = session.sender();
        Self {
            session: Arc::new(tokio::sync::Mutex::new(session)),
            sender,
        }
    }

    async fn send(&self, payload: Arc<[u8]>) -> Result<(), String> {
        match &self.sender {
            Some(sender) => sender.send(payload).await,
            None => self.session.lock().await.send(payload).await,
        }
    }

    async fn send_object(&self, object: Arc<[u8]>) -> Result<(), String> {
        match &self.sender {
            Some(sender) => sender.send_object(object).await,
            None => self.session.lock().await.send_object(object).await,
        }
    }
}

/// Per-peer cancellation slot for the receive or reconnect task.
///
/// The generation identifies the registration that created it, so a session
/// task exiting later removes only its own slot: a newer registration for the
/// same peer is kept, and dropping a superseded sender stops the older task.
pub(crate) struct CancelRegistration {
    generation: u64,
    tx: oneshot::Sender<()>,
}

/// Drops the cancellation slot only when it still belongs to `generation`.
pub(crate) async fn release_cancel(
    cancellations: &Arc<RwLock<HashMap<PeerId, CancelRegistration>>>,
    peer: PeerId,
    generation: u64,
) {
    let mut guard = cancellations.write().await;
    if guard
        .get(&peer)
        .is_some_and(|entry| entry.generation == generation)
    {
        guard.remove(&peer);
    }
}

impl PeerManager {
    pub fn new(config: OverlayConfig) -> Result<Self, OverlayError> {
        Self::with_overlay(config, OverlayId::from_bytes([0; 32]))
    }

    pub fn with_overlay(config: OverlayConfig, overlay: OverlayId) -> Result<Self, OverlayError> {
        let (shutdown, _) = watch::channel(false);
        Ok(Self {
            pool: Arc::new(OverlayPeerPool::new(config)?),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            scores: Arc::new(RwLock::new(HashMap::new())),
            cancellations: Arc::new(RwLock::new(HashMap::new())),
            cancel_seq: Arc::new(AtomicU64::new(0)),
            shutdown,
            overlay,
            tasks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        })
    }

    pub fn pool(&self) -> Arc<OverlayPeerPool> {
        self.pool.clone()
    }

    /// Subscribes to connection and failure notifications from the manager.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.pool.subscribe_statuses()
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "The signature intentionally mirrors protocol flags and values moved across async task boundaries"
    )]
    pub async fn add_session(&self, session: Box<dyn OverlaySession>) {
        let peer = session.peer_id();
        if self.sessions.read().await.contains_key(&peer) {
            let _ = self
                .pool
                .report_status(PeerStatus::Reconnecting { peer, attempt: 0 });
            return;
        }
        let (generation, mut cancel_rx) = self.register_cancel(peer).await;
        let shared = SharedSession::wrap(session);
        self.sessions.write().await.insert(peer, shared.clone());
        self.scores.write().await.entry(peer).or_insert(0);
        self.pool.register_peer(peer).await;
        let pool = self.pool.clone();
        let overlay = self.overlay;
        let idle_timeout = pool.config.peer_idle_timeout;
        let mut shutdown = self.shutdown.subscribe();
        let sessions = self.sessions.clone();
        let scores = self.scores.clone();
        let cancellations = self.cancellations.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = &mut cancel_rx => break,
                    packet = async {
                        tokio::time::timeout(idle_timeout, async {
                            shared.session.lock().await.receive().await
                        }).await
                    } => {
                        match packet {
                            Ok(Ok(payload)) => {
                                let packet = OverlayPacket { payload, routing: RoutingMetadata::new(overlay, peer) };
                                // Bounded wait: a stalled consumer must
                                // never freeze this loop, because the
                                // inline query answers it serves are what
                                // keeps peers listing this node as a
                                // verified member.
                                match tokio::time::timeout(
                                    INGEST_WAIT_TIMEOUT,
                                    pool.ingest(packet),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {}
                                    Ok(Err(_)) => break,
                                    Err(_) => {
                                        let drops = pool.record_ingest_drop();
                                        if drops == 1 || drops.is_multiple_of(100) {
                                            log::warn!(
                                                "overlay ingest queue saturated, dropping packet: peer={peer:?} drops={drops}"
                                            );
                                        } else {
                                            log::debug!(
                                                "overlay ingest queue saturated: peer={peer:?} drops={drops}"
                                            );
                                        }
                                    }
                                }
                                // The peer delivered the packet; whether
                                // our queue could take it is not peer
                                // quality.
                                scores.write().await.entry(peer).and_modify(|score| *score += 1);
                            }
                            Ok(Err(_)) => {
                                let _ = pool.report_status(PeerStatus::Failed { peer });
                                break;
                            }
                            Err(_) => {
                                // The idle deadline bounds one `receive()` call.  Keep the
                                // session while it still sees transport traffic, which is
                                // consumed inside `receive()` and never completes it.
                                if shared
                                    .session
                                    .lock()
                                    .await
                                    .last_activity()
                                    .is_some_and(|at| at.elapsed() < idle_timeout)
                                {
                                    log::debug!(
                                        "overlay peer session kept alive by protocol traffic: peer={peer:?}"
                                    );
                                    continue;
                                }
                                let _ = pool.report_status(PeerStatus::Failed { peer });
                                break;
                            }
                        }
                    }
                }
            }
            // Unregister only while this task still owns the registered
            // session: `remove_session` may already have replaced or removed
            // it, and a re-added session must keep its own registration.
            let owned = {
                let mut guard = sessions.write().await;
                let owned = guard
                    .get(&peer)
                    .is_some_and(|stored| Arc::ptr_eq(&stored.session, &shared.session));
                if owned {
                    guard.remove(&peer);
                }
                owned
            };
            if owned {
                pool.unregister_peer(peer).await;
                scores
                    .write()
                    .await
                    .entry(peer)
                    .and_modify(|score| *score -= 1);
            }
            release_cancel(&cancellations, peer, generation).await;
        });
        self.tasks.lock().await.push(task);
    }

    /// Registers this manager's cancellation slot for `peer`.
    ///
    /// Replacing a live registration drops the previous sender, which stops a
    /// superseded receive or reconnect task that has not observed its slot yet.
    async fn register_cancel(&self, peer: PeerId) -> (u64, oneshot::Receiver<()>) {
        let generation = self.cancel_seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.cancellations
            .write()
            .await
            .insert(peer, CancelRegistration { generation, tx });
        (generation, rx)
    }

    /// Removes a peer's session and stops its receive or reconnect task.
    ///
    /// Returns `true` when a live session was removed.  Fixed seed sets
    /// (validators, configured ADNL addresses) must never go through this
    /// path: quality-based eviction only considers non-pinned peers.
    pub async fn remove_session(&self, peer: PeerId) -> bool {
        let cancel = self.cancellations.write().await.remove(&peer);
        let removed = self.sessions.write().await.remove(&peer).is_some();
        self.scores.write().await.remove(&peer);
        if removed {
            // The session task wakes on its cancellation slot and skips its
            // own cleanup (`Arc::ptr_eq` no longer matches), so `Disconnected`
            // is reported exactly once, here.
            self.pool.unregister_peer(peer).await;
        }
        if let Some(entry) = cancel {
            let _ = entry.tx.send(());
        }
        removed
    }

    /// Returns the peer IDs of every currently registered session.
    pub async fn peers(&self) -> Vec<PeerId> {
        self.sessions.read().await.keys().copied().collect()
    }

    #[expect(
        clippy::too_many_lines,
        reason = "The signature intentionally mirrors protocol flags and values moved across async task boundaries"
    )]
    pub async fn add_session_with_reconnect(
        &self,
        session: Box<dyn OverlaySession>,
        reconnect: ReconnectFactory,
        max_attempts: u32,
        base_backoff: Duration,
    ) {
        let peer = session.peer_id();
        if self.sessions.read().await.contains_key(&peer) {
            return;
        }
        let (generation, mut cancel_rx) = self.register_cancel(peer).await;
        let sessions = self.sessions.clone();
        let scores = self.scores.clone();
        let cancellations = self.cancellations.clone();
        let pool = self.pool.clone();
        let overlay = self.overlay;
        let idle_timeout = pool.config.peer_idle_timeout;
        let mut shutdown = self.shutdown.subscribe();
        let task = tokio::spawn(async move {
            let mut current = session;
            let mut cancelled = false;
            'reconnect: loop {
                let shared = SharedSession::wrap(current);
                sessions.write().await.insert(peer, shared.clone());
                pool.register_peer(peer).await;
                let _ = pool.report_status(PeerStatus::Connected { peer });
                log::debug!("overlay peer session active: peer={peer:?}");
                let mut stopped = false;
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        _ = &mut cancel_rx => { cancelled = true; break; }
                        packet = async {
                            tokio::time::timeout(idle_timeout, async {
                                shared.session.lock().await.receive().await
                            }).await
                        } => {
                            match packet {
                                Ok(Ok(payload)) => {
                                    let packet = OverlayPacket { payload, routing: RoutingMetadata::new(overlay, peer) };
                                    if pool.ingest(packet).await.is_err() {
                                        stopped = true;
                                        break;
                                    }
                                    scores.write().await.entry(peer).and_modify(|score| *score += 1);
                                }
                                Ok(Err(_)) => {
                                    log::warn!("overlay peer session lost: peer={peer:?}");
                                    break;
                                }
                                Err(_) => {
                                    // Same idle-deadline rule as `add_session`: protocol
                                    // traffic is consumed inside `receive()` and must keep a
                                    // healthy session alive across the deadline.
                                    if shared
                                        .session
                                        .lock()
                                        .await
                                        .last_activity()
                                        .is_some_and(|at| at.elapsed() < idle_timeout)
                                    {
                                        continue;
                                    }
                                    log::warn!("overlay peer session lost: peer={peer:?}");
                                    break;
                                }
                            }
                        }
                    }
                }
                // Unregister only while this task still owns the registered
                // session: `remove_session` or a superseding re-add must keep
                // its own registration intact.
                let owned = {
                    let mut guard = sessions.write().await;
                    let owned = guard
                        .get(&peer)
                        .is_some_and(|stored| Arc::ptr_eq(&stored.session, &shared.session));
                    if owned {
                        guard.remove(&peer);
                    }
                    owned
                };
                if owned {
                    pool.unregister_peer(peer).await;
                }
                if cancelled {
                    // `remove_session` already reported `Disconnected`.
                    break 'reconnect;
                }
                if stopped {
                    // The pool is closed; the caller is shutting down.
                    break 'reconnect;
                }
                if *shutdown.borrow() {
                    let _ = pool.report_status(PeerStatus::Disconnected { peer });
                    break 'reconnect;
                }
                if max_attempts == 0 {
                    let _ = pool.report_status(PeerStatus::Failed { peer });
                    break 'reconnect;
                }
                let _ = pool.report_status(PeerStatus::Failed { peer });
                let mut replacement = None;
                for attempt in 1..=max_attempts {
                    let _ = pool.report_status(PeerStatus::Reconnecting { peer, attempt });
                    log::debug!(
                        "overlay reconnect attempt {attempt}/{max_attempts}: peer={peer:?}"
                    );
                    let multiplier = 1u32
                        .checked_shl(attempt.saturating_sub(1))
                        .unwrap_or(u32::MAX);
                    let delay = base_backoff
                        .saturating_mul(multiplier)
                        .min(Duration::from_secs(30));
                    tokio::select! {
                        _ = shutdown.changed() => break 'reconnect,
                        _ = &mut cancel_rx => { cancelled = true; break; }
                        () = tokio::time::sleep(delay) => {}
                    }
                    if cancelled {
                        break 'reconnect;
                    }
                    match reconnect(peer).await {
                        Ok(session) => {
                            replacement = Some(session);
                            break;
                        }
                        Err(error) => log::warn!("overlay reconnect for {peer:?} failed: {error}"),
                    }
                }
                if cancelled {
                    break 'reconnect;
                }
                // A cancellation observed while `reconnect()` was in flight
                // still wins: drop the replacement instead of reviving the
                // session for an evicted peer.
                match cancel_rx.try_recv() {
                    Ok(()) | Err(oneshot::error::TryRecvError::Closed) => {
                        cancelled = true;
                        break 'reconnect;
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {}
                }
                if *shutdown.borrow() {
                    let _ = pool.report_status(PeerStatus::Disconnected { peer });
                    break 'reconnect;
                }
                let Some(session) = replacement else {
                    // Reconnect attempts are exhausted.
                    break 'reconnect;
                };
                current = session;
            }
            if !cancelled {
                release_cancel(&cancellations, peer, generation).await;
            }
        });
        self.tasks.lock().await.push(task);
    }

    pub async fn broadcast(&self, payload: Arc<[u8]>) -> usize {
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let results = futures::future::join_all(sessions.into_iter().map(|session| {
            let payload = payload.clone();
            async move { session.send(payload).await }
        }))
        .await;
        results.into_iter().flatten().count()
    }

    /// Broadcasts a raw overlay object to every connected session.
    ///
    /// Unlike [`Self::broadcast`], which wraps the payload in
    /// `overlay.unicast`, the object rides after the
    /// `overlay.message` header as-is - the framing overlay
    /// broadcasts require.  Returns the number of sessions the
    /// object was handed to.
    pub async fn broadcast_object(&self, object: Arc<[u8]>) -> usize {
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let results = futures::future::join_all(sessions.into_iter().map(|session| {
            let object = object.clone();
            async move { session.send_object(object).await }
        }))
        .await;
        results.into_iter().flatten().count()
    }

    /// Broadcasts a raw overlay object, starting sends to `ranked` peers first.
    ///
    /// `join_all` polls its futures in submission order, so the sends toward
    /// the highest-ranked (best delivery quality) peers are issued before the
    /// rest.  Sessions absent from `ranked` still receive the object in the
    /// same call - ordering adds no latency for them.  Returns the number of
    /// sessions the object was handed to.
    pub async fn broadcast_object_in_order(&self, object: Arc<[u8]>, ranked: &[PeerId]) -> usize {
        let sessions = self.sessions.read().await;
        let mut ordered: Vec<SharedSession> = Vec::with_capacity(sessions.len());
        let mut seen = HashSet::with_capacity(sessions.len());
        for peer in ranked {
            if let Some(session) = sessions.get(peer)
                && seen.insert(*peer)
            {
                ordered.push(session.clone());
            }
        }
        for (peer, session) in sessions.iter() {
            if seen.insert(*peer) {
                ordered.push(session.clone());
            }
        }
        drop(sessions);
        let results = futures::future::join_all(ordered.into_iter().map(|session| {
            let object = object.clone();
            async move { session.send_object(object).await }
        }))
        .await;
        results.into_iter().flatten().count()
    }

    pub async fn score(&self, peer: PeerId) -> i32 {
        self.scores
            .read()
            .await
            .get(&peer)
            .copied()
            .unwrap_or_default()
    }

    pub async fn peer_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    pub async fn shutdown_wait(&self) {
        self.shutdown();
        let tasks = std::mem::take(&mut *self.tasks.lock().await);
        for task in tasks {
            let _ = task.await;
        }
    }
}
