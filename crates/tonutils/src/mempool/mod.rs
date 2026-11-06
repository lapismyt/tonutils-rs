//! External-message validation, deduplication, and event delivery.
//!
//! The mempool layer deliberately does not own transport, discovery, peer
//! management, or DHT publication. Build an [`crate::overlay::OverlayClient`]
//! (or server) first and hand it to [`MempoolServer`]. Inbound overlay packets
//! are reduced to external-message BoCs, validated, deduplicated, and emitted
//! as [`MempoolEvent`] values. [`MempoolClient::send_external`] validates the
//! local copy and asks the overlay client to broadcast it.

mod client;
mod dedup;
mod server;
mod wire;

#[cfg(test)]
mod tests;

pub use client::MempoolClient;
pub use server::MempoolServer;

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{
    overlay::{PeerStatus, RoutingMetadata},
    tlb::Message,
};

use futures::stream::Stream;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::mpsc;

/// Hash of a serialized external message.
pub type MessageHash = [u8; 32];

/// Mempool resource and validation limits.
#[derive(Clone, Debug)]
pub struct MempoolConfig {
    pub event_queue_capacity: usize,
    pub max_message_size: usize,
    pub dedup_shards: usize,
    pub require_boc_magic: bool,
    pub validate_message: bool,
    pub dedup_ttl: Duration,
    pub max_dedup_entries: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            event_queue_capacity: 1024,
            max_message_size: 1 << 20,
            dedup_shards: 32,
            require_boc_magic: true,
            validate_message: true,
            dedup_ttl: Duration::from_secs(300),
            max_dedup_entries: 100_000,
        }
    }
}

/// Events emitted by [`MempoolServer`].
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

impl MempoolEvent {
    /// Returns a lazy view for an accepted external message event.
    pub fn lazy_message(&self) -> Option<LazyExternalMessage> {
        match self {
            Self::ExternalMessage { hash, raw_boc, .. } => {
                Some(LazyExternalMessage::new(*hash, raw_boc.clone()))
            }
            _ => None,
        }
    }
}

/// Raw external message whose typed TL-B representation is decoded on demand.
#[derive(Clone, Debug)]
pub struct LazyExternalMessage {
    hash: MessageHash,
    raw_boc: Arc<[u8]>,
}

impl LazyExternalMessage {
    pub fn new(hash: MessageHash, raw_boc: Arc<[u8]>) -> Self {
        Self { hash, raw_boc }
    }

    pub fn hash(&self) -> MessageHash {
        self.hash
    }

    pub fn raw_boc(&self) -> Arc<[u8]> {
        self.raw_boc.clone()
    }

    pub fn decode(&self) -> Result<Message, MempoolError> {
        wire::decode_external(&self.raw_boc)
    }

    pub fn destination(&self) -> Result<Option<[u8; 32]>, MempoolError> {
        wire::destination(&self.raw_boc)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MempoolMetrics {
    pub accepted: u64,
    pub duplicates: u64,
    pub rejected: u64,
    pub broadcast_failures: u64,
    pub invalid_warnings: u64,
    pub overlay_packets: u64,
    /// Packets dropped by the overlay pool before the mempool consumed them.
    pub queue_full_drops: u64,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MempoolError {
    #[error("mempool event queue capacity must be greater than zero")]
    InvalidQueueCapacity,
    #[error("mempool dedup shard count must be greater than zero")]
    InvalidShardCount,
    #[error("mempool dedup entry limit must be greater than zero")]
    InvalidDedupCapacity,
    #[error("external message is empty or smaller than its envelope")]
    InvalidEnvelope,
    #[error("external message is not a serialized BoC")]
    InvalidBoc,
    #[error("external BoC is not a valid external message")]
    InvalidMessage,
    #[error("external message exceeds configured size limit")]
    MessageTooLarge,
    #[error("mempool event queue is closed")]
    QueueClosed,
    #[error("external message failed lazy TL-B decoding: {0}")]
    Decode(String),
    #[error("external message broadcast failed: {0}")]
    Broadcast(String),
}

/// Extracts the deliverable external-message BoC from an overlay payload.
pub(crate) fn unwrap_external_message_payload(payload: Vec<u8>) -> Vec<u8> {
    wire::unwrap_external_message_payload(payload)
}

fn hash_boc(raw_boc: &[u8]) -> MessageHash {
    Sha256::digest(raw_boc).into()
}

fn event_stream(
    receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<MempoolEvent>>>,
) -> impl Stream<Item = MempoolEvent> + use<> {
    futures::stream::unfold(receiver, |receiver| async move {
        let event = receiver.lock().await.recv().await?;
        Some((event, receiver))
    })
}
