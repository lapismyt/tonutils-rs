//! Shared external-message client.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use futures::{Stream, StreamExt};
use tokio::sync::{Mutex, mpsc};

use crate::overlay::{
    OverlayClient, OverlayId, PeerId, PeerStatus, RoutingMetadata,
    tl::TonNodeExternalMessageBroadcast,
};

use super::{
    LazyExternalMessage, MempoolConfig, MempoolError, MempoolEvent, MempoolMetrics, MessageHash,
    dedup::Dedup, event_stream, hash_boc, wire,
};

pub(crate) struct MempoolCore {
    pub(crate) config: MempoolConfig,
    pub(crate) overlay: OverlayClient,
    pub(crate) event_tx: mpsc::Sender<MempoolEvent>,
    pub(crate) event_rx: Arc<Mutex<mpsc::Receiver<MempoolEvent>>>,
    pub(crate) dedup: Dedup,
    pub(crate) accepted: AtomicU64,
    pub(crate) duplicates: Arc<AtomicU64>,
    pub(crate) rejected: AtomicU64,
    pub(crate) broadcast_failures: AtomicU64,
    pub(crate) invalid_warnings: AtomicU64,
    pub(crate) overlay_packets: AtomicU64,
    pub(crate) queue_full_drops: AtomicU64,
}

impl MempoolCore {
    pub(crate) fn new(overlay: OverlayClient, config: MempoolConfig) -> Result<Self, MempoolError> {
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
        let duplicates = Arc::new(AtomicU64::new(0));
        let dedup = Dedup::new(config.clone(), Arc::clone(&duplicates));
        Ok(Self {
            config,
            overlay,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            dedup,
            accepted: AtomicU64::new(0),
            duplicates,
            rejected: AtomicU64::new(0),
            broadcast_failures: AtomicU64::new(0),
            invalid_warnings: AtomicU64::new(0),
            overlay_packets: AtomicU64::new(0),
            queue_full_drops: AtomicU64::new(0),
        })
    }

    pub(crate) async fn accept(
        &self,
        raw_boc: Arc<[u8]>,
        routing: RoutingMetadata,
        local: bool,
    ) -> Result<Option<MessageHash>, MempoolError> {
        let hash = hash_boc(&raw_boc);
        let destination = match wire::validate_envelope(&raw_boc, &self.config) {
            Ok(destination) => destination,
            Err(error) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        if !self.dedup.accept(hash).await {
            return Ok(None);
        }
        self.accepted.fetch_add(1, Ordering::Relaxed);
        let event = MempoolEvent::ExternalMessage {
            hash,
            raw_boc,
            destination,
            routing,
            timestamp: std::time::SystemTime::now(),
        };
        if local {
            match self.event_tx.try_send(event) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::warn!("mempool event queue full, dropping local echo: hash={hash:02x?}");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(MempoolError::QueueClosed);
                }
            }
        } else {
            self.event_tx
                .send(event)
                .await
                .map_err(|_| MempoolError::QueueClosed)?;
        }
        Ok(Some(hash))
    }

    pub(crate) fn metrics(&self) -> MempoolMetrics {
        MempoolMetrics {
            accepted: self.accepted.load(Ordering::Relaxed),
            duplicates: self.duplicates.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            broadcast_failures: self.broadcast_failures.load(Ordering::Relaxed),
            invalid_warnings: self.invalid_warnings.load(Ordering::Relaxed),
            overlay_packets: self.overlay_packets.load(Ordering::Relaxed),
            queue_full_drops: self.queue_full_drops.load(Ordering::Relaxed),
        }
    }
}

/// Shared external-message handle returned by [`super::MempoolServer::new`].
#[derive(Clone)]
pub struct MempoolClient {
    pub(crate) core: Arc<MempoolCore>,
}

impl MempoolClient {
    pub(crate) fn new(core: Arc<MempoolCore>) -> Self {
        Self { core }
    }

    /// Returns the overlay client used for submissions.
    pub fn overlay(&self) -> OverlayClient {
        self.core.overlay.clone()
    }

    /// Validates, deduplicates, emits a local event, and broadcasts one
    /// external-message BoC through the legacy signed overlay broadcast path.
    ///
    /// TON external messages are not generic Plumtree payloads: their wire
    /// object is [`crate::overlay::tl::TonNodeExternalMessageBroadcast`], and
    /// this method intentionally calls
    /// [`OverlayClient::broadcast_signed`] rather than the Plumtree simple/FEC
    /// methods. The returned hash means the local mempool accepted the BoC;
    /// it does not prove network delivery, inclusion, or finality.
    pub async fn send_external(
        &self,
        raw_boc: impl Into<Arc<[u8]>>,
    ) -> Result<MessageHash, MempoolError> {
        let started = std::time::Instant::now();
        let raw_boc = raw_boc.into();
        let hash = hash_boc(&raw_boc);
        let routing =
            RoutingMetadata::new(OverlayId::from_name(b"local"), PeerId::from_bytes([0; 32]));
        let Some(hash) = self.core.accept(raw_boc.clone(), routing, true).await? else {
            return Ok(hash);
        };
        let payload = tl_proto::serialize(TonNodeExternalMessageBroadcast {
            message: crate::overlay::tl::TonNodeExternalMessage {
                data: raw_boc.to_vec(),
            },
        });
        match self
            .core
            .overlay
            .broadcast_signed(Arc::from(payload), 0)
            .await
        {
            Ok(sent) => log::info!(
                "mempool external message submitted: hash={hash:02x?} bytes={} sessions={sent} elapsed={:?}",
                raw_boc.len(),
                started.elapsed(),
            ),
            Err(error) => {
                self.core.broadcast_failures.fetch_add(1, Ordering::Relaxed);
                log::warn!(
                    "mempool external message broadcast failed: hash={hash:02x?} error={error}"
                );
                return Err(MempoolError::Broadcast(error.to_string()));
            }
        }
        Ok(hash)
    }

    /// Returns the mempool counters.
    pub fn metrics(&self) -> MempoolMetrics {
        self.core.metrics()
    }

    /// Emits a lazy view over accepted external-message data.
    pub fn lazy_message(&self, hash: MessageHash, raw_boc: Arc<[u8]>) -> LazyExternalMessage {
        LazyExternalMessage::new(hash, raw_boc)
    }

    /// Returns the shared event stream.
    pub fn events(&self) -> impl Stream<Item = MempoolEvent> + use<> {
        event_stream(Arc::clone(&self.core.event_rx))
    }

    /// Runs `handler` for every event until the event queue closes.
    pub async fn run_handler<H, F>(&self, mut handler: H)
    where
        H: FnMut(MempoolEvent) -> F,
        F: std::future::Future<Output = ()>,
    {
        let mut events = Box::pin(self.events());
        while let Some(event) = events.next().await {
            handler(event).await;
        }
    }

    /// Emits a later inclusion result without changing the initial
    /// external-message event.
    pub async fn mark_included(
        &self,
        hash: MessageHash,
        block: impl Into<Arc<[u8]>>,
        transaction: Option<Arc<[u8]>>,
    ) -> Result<(), MempoolError> {
        self.core
            .event_tx
            .send(MempoolEvent::Included {
                hash,
                block: block.into(),
                transaction,
            })
            .await
            .map_err(|_| MempoolError::QueueClosed)
    }

    /// Emits a peer lifecycle event.
    pub async fn peer_status(&self, status: PeerStatus) -> Result<(), MempoolError> {
        self.core
            .event_tx
            .send(MempoolEvent::PeerStatus(status))
            .await
            .map_err(|_| MempoolError::QueueClosed)
    }

    /// Accepts an inbound external-message payload for tests and custom
    /// transports.
    pub async fn ingest(
        &self,
        raw_boc: impl Into<Arc<[u8]>>,
        routing: RoutingMetadata,
    ) -> Result<Option<MessageHash>, MempoolError> {
        self.core.accept(raw_boc.into(), routing, false).await
    }

    /// Returns a local-routing marker for callers that ingest without one.
    pub fn local_routing() -> RoutingMetadata {
        RoutingMetadata::new(OverlayId::from_name(b"local"), PeerId::from_bytes([0; 32]))
    }
}
