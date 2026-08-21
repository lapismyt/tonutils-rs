//! Building blocks for TON overlay networks.
//!
//! This crate deliberately separates protocol-neutral peer management from
//! discovery and wire schemas. It provides bounded fan-out, peer scoring, and
//! routing metadata that can be driven by an ADNL session implementation.
//! DHT and overlay packet schemas are not guessed here; they are tracked in
//! `docs/reference/network/overlay.md` until upstream fixtures are available.

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[cfg(feature = "runtime")]
use tokio::sync::{RwLock, broadcast, mpsc};

/// A 256-bit overlay identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OverlayId([u8; 32]);

impl OverlayId {
    /// Derives an overlay ID from its canonical name.
    pub fn from_name(name: &[u8]) -> Self {
        Self(Sha256::digest(name).into())
    }

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for OverlayId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Stable identifier for an overlay peer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Metadata attached by the receive path without decoding the payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingMetadata {
    pub overlay: OverlayId,
    pub peer: PeerId,
    pub received_at: SystemTime,
    pub hop: u8,
}

impl RoutingMetadata {
    pub fn new(overlay: OverlayId, peer: PeerId) -> Self {
        Self {
            overlay,
            peer,
            received_at: SystemTime::now(),
            hop: 0,
        }
    }

    pub fn received_unix_millis(&self) -> u128 {
        self.received_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    }
}

/// State changes emitted by the peer pool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerStatus {
    Connected { peer: PeerId },
    Disconnected { peer: PeerId },
    Failed { peer: PeerId },
}

/// A bounded packet accepted by the overlay receive path.
#[derive(Clone, Debug)]
pub struct OverlayPacket {
    pub payload: Arc<[u8]>,
    pub routing: RoutingMetadata,
}

/// Configuration for a bounded overlay peer pool.
#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub queue_capacity: usize,
    pub max_packet_size: usize,
    pub peer_idle_timeout: Duration,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            max_packet_size: 1 << 20,
            peer_idle_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Error)]
pub enum OverlayError {
    #[error("overlay queue capacity must be greater than zero")]
    InvalidCapacity,
    #[error("overlay packet exceeds configured size limit")]
    PacketTooLarge,
    #[error("overlay peer is not registered")]
    UnknownPeer,
    #[error("overlay receive queue is closed")]
    QueueClosed,
}

/// Bounded peer pool used by scanners and broadcasters.
#[cfg(feature = "runtime")]
pub struct OverlayPeerPool {
    config: OverlayConfig,
    packets: mpsc::Sender<OverlayPacket>,
    packet_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<OverlayPacket>>>,
    statuses: broadcast::Sender<PeerStatus>,
    peers: Arc<RwLock<HashSet<PeerId>>>,
}

#[cfg(feature = "runtime")]
impl OverlayPeerPool {
    pub fn new(config: OverlayConfig) -> Result<Self, OverlayError> {
        if config.queue_capacity == 0 {
            return Err(OverlayError::InvalidCapacity);
        }
        let (packets, packet_rx) = mpsc::channel(config.queue_capacity);
        let (statuses, _) = broadcast::channel(config.queue_capacity);
        Ok(Self {
            config,
            packets,
            packet_rx: Arc::new(tokio::sync::Mutex::new(packet_rx)),
            statuses,
            peers: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    pub async fn register_peer(&self, peer: PeerId) {
        self.peers.write().await.insert(peer);
        let _ = self.statuses.send(PeerStatus::Connected { peer });
    }

    pub async fn unregister_peer(&self, peer: PeerId) {
        self.peers.write().await.remove(&peer);
        let _ = self.statuses.send(PeerStatus::Disconnected { peer });
    }

    pub async fn ingest(&self, packet: OverlayPacket) -> Result<(), OverlayError> {
        if packet.payload.len() > self.config.max_packet_size {
            return Err(OverlayError::PacketTooLarge);
        }
        if !self.peers.read().await.contains(&packet.routing.peer) {
            return Err(OverlayError::UnknownPeer);
        }
        self.packets
            .send(packet)
            .await
            .map_err(|_| OverlayError::QueueClosed)
    }

    pub async fn next_packet(&self) -> Option<OverlayPacket> {
        self.packet_rx.lock().await.recv().await
    }

    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.statuses.subscribe()
    }

    pub fn packet_sender(&self) -> mpsc::Sender<OverlayPacket> {
        self.packets.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unknown_and_oversized_packets() {
        let pool = OverlayPeerPool::new(OverlayConfig {
            max_packet_size: 2,
            ..Default::default()
        })
        .unwrap();
        let packet = OverlayPacket {
            payload: Arc::from([1, 2, 3].as_slice()),
            routing: RoutingMetadata::new(
                OverlayId::from_name(b"test"),
                PeerId::from_bytes([1; 32]),
            ),
        };
        assert!(matches!(
            pool.ingest(packet).await,
            Err(OverlayError::PacketTooLarge)
        ));
        pool.register_peer(PeerId::from_bytes([1; 32])).await;
        let packet = OverlayPacket {
            payload: Arc::from([1].as_slice()),
            routing: RoutingMetadata::new(
                OverlayId::from_name(b"test"),
                PeerId::from_bytes([1; 32]),
            ),
        };
        pool.ingest(packet).await.unwrap();
        assert_eq!(pool.next_packet().await.unwrap().payload.as_ref(), [1]);
    }
}
