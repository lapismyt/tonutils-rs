//! Low-latency pending external-message scanner primitives.
//!
//! The fast path validates the outer BoC envelope, hashes the shared raw
//! bytes, performs bounded deduplication, and publishes an event. Full TL-B
//! decoding, storage, and LiteServer inclusion queries are intentionally left
//! to consumers or a future slow-path worker.

use futures::future::{BoxFuture, join_all};
use futures::stream::{self, Stream};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use tonutils_overlay::{OverlayId, PeerId, PeerStatus, RoutingMetadata};

/// Hash of a serialized external message.
pub type MessageHash = [u8; 32];

/// Scanner resource and validation limits.
#[derive(Clone, Debug)]
pub struct MempoolConfig {
    pub event_queue_capacity: usize,
    pub max_message_size: usize,
    pub dedup_shards: usize,
    pub require_boc_magic: bool,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            event_queue_capacity: 1024,
            max_message_size: 1 << 20,
            dedup_shards: 32,
            require_boc_magic: true,
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

/// A destination that can accept a raw external message for broadcast.
pub trait BroadcastPeer: Send + Sync {
    fn send_external(&self, raw_boc: Arc<[u8]>) -> BoxFuture<'static, Result<(), String>>;
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MempoolError {
    #[error("mempool event queue capacity must be greater than zero")]
    InvalidQueueCapacity,
    #[error("mempool dedup shard count must be greater than zero")]
    InvalidShardCount,
    #[error("external message is empty or smaller than its envelope")]
    InvalidEnvelope,
    #[error("external message is not a serialized BoC")]
    InvalidBoc,
    #[error("external message exceeds configured size limit")]
    MessageTooLarge,
    #[error("mempool event queue is closed")]
    QueueClosed,
}

/// A bounded, deduplicating scanner.
pub struct MempoolScanner {
    config: MempoolConfig,
    event_tx: mpsc::Sender<MempoolEvent>,
    event_rx: Arc<Mutex<mpsc::Receiver<MempoolEvent>>>,
    dedup: Arc<Vec<Mutex<HashSet<MessageHash>>>>,
    broadcast_peers: Arc<Mutex<Vec<Arc<dyn BroadcastPeer>>>>,
}

impl MempoolScanner {
    pub fn new(config: MempoolConfig) -> Result<Self, MempoolError> {
        if config.event_queue_capacity == 0 {
            return Err(MempoolError::InvalidQueueCapacity);
        }
        if config.dedup_shards == 0 {
            return Err(MempoolError::InvalidShardCount);
        }
        let (event_tx, event_rx) = mpsc::channel(config.event_queue_capacity);
        let dedup = (0..config.dedup_shards)
            .map(|_| Mutex::new(HashSet::new()))
            .collect();
        Ok(Self {
            config,
            event_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            dedup: Arc::new(dedup),
            broadcast_peers: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Returns the scanner's Rust stream. The receiver is shared so ingest and
    /// outbound broadcast can continue while a consumer is polling the stream.
    pub fn events(&self) -> impl Stream<Item = MempoolEvent> {
        let receiver = self.event_rx.clone();
        stream::unfold(receiver, |receiver| async move {
            let event = receiver.lock().await.recv().await?;
            Some((event, receiver))
        })
    }

    pub async fn add_broadcast_peer(&self, peer: Arc<dyn BroadcastPeer>) {
        self.broadcast_peers.lock().await.push(peer);
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
            let results = join_all(
                peers
                    .into_iter()
                    .map(|peer| peer.send_external(raw_boc.clone())),
            )
            .await;
            let _failed_broadcasts = results.iter().filter(|result| result.is_err()).count();
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

    async fn accept_fast(
        &self,
        raw_boc: Arc<[u8]>,
        routing: RoutingMetadata,
    ) -> Result<Option<MessageHash>, MempoolError> {
        validate_envelope(&raw_boc, &self.config)?;
        let hash: MessageHash = Sha256::digest(&raw_boc).into();
        let shard = usize::from(hash[0]) % self.dedup.len();
        if !self.dedup[shard].lock().await.insert(hash) {
            return Ok(None);
        }
        self.event_tx
            .send(MempoolEvent::ExternalMessage {
                hash,
                raw_boc,
                destination: None,
                routing,
                timestamp: SystemTime::now(),
            })
            .await
            .map_err(|_| MempoolError::QueueClosed)?;
        Ok(Some(hash))
    }
}

fn validate_envelope(raw_boc: &[u8], config: &MempoolConfig) -> Result<(), MempoolError> {
    if raw_boc.len() < 4 {
        return Err(MempoolError::InvalidEnvelope);
    }
    if raw_boc.len() > config.max_message_size {
        return Err(MempoolError::MessageTooLarge);
    }
    if config.require_boc_magic && raw_boc[..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err(MempoolError::InvalidBoc);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn boc(body: u8) -> Vec<u8> {
        vec![0xb5, 0xee, 0x9c, 0x72, body]
    }

    #[tokio::test]
    async fn publishes_once_for_duplicate_peers() {
        let scanner = MempoolScanner::new(MempoolConfig::default()).unwrap();
        let mut events = Box::pin(scanner.events());
        let routing =
            RoutingMetadata::new(OverlayId::from_name(b"test"), PeerId::from_bytes([1; 32]));
        let raw = boc(7);
        assert!(
            scanner
                .ingest(raw.clone(), routing.clone())
                .await
                .unwrap()
                .is_some()
        );
        assert!(scanner.ingest(raw, routing).await.unwrap().is_none());
        assert!(matches!(
            events.next().await,
            Some(MempoolEvent::ExternalMessage { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_non_boc_fast() {
        let scanner = MempoolScanner::new(MempoolConfig::default()).unwrap();
        assert_eq!(
            scanner
                .ingest(
                    vec![1, 2, 3, 4],
                    RoutingMetadata::new(
                        OverlayId::from_name(b"test"),
                        PeerId::from_bytes([1; 32])
                    )
                )
                .await,
            Err(MempoolError::InvalidBoc)
        );
    }
}
