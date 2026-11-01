//! High-level overlay server handle.

use std::sync::Arc;

use tokio::sync::Mutex;

use super::{
    DiscoveryStats, OverlayClient, OverlayError, OverlayPacket, PeerId, PeerStatus,
    PlumtreeBroadcastId, PlumtreeStats, ProtocolStats,
};

/// An overlay node that owns inbound-serving state such as DHT address
/// publication in addition to the client transport.
///
/// The underlying [`OverlayClient`] is available through [`Self::client`], so
/// an application protocol can consume packets while the server keeps its
/// publication and peer-maintenance tasks alive.
#[derive(Clone)]
pub struct OverlayServer {
    client: OverlayClient,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl OverlayServer {
    pub(crate) fn new(client: OverlayClient, tasks: Vec<tokio::task::JoinHandle<()>>) -> Self {
        Self {
            client,
            tasks: Arc::new(Mutex::new(tasks)),
        }
    }

    /// Returns the shared client handle.
    pub fn client(&self) -> OverlayClient {
        self.client.clone()
    }

    pub fn subscribe_packets(&self) -> tokio::sync::broadcast::Receiver<OverlayPacket> {
        self.client.subscribe_packets()
    }

    pub fn subscribe_statuses(&self) -> tokio::sync::broadcast::Receiver<PeerStatus> {
        self.client.subscribe_statuses()
    }

    pub fn discovery_stats(&self) -> DiscoveryStats {
        self.client.discovery_stats()
    }

    pub fn protocol_stats(&self) -> ProtocolStats {
        self.client.protocol_stats()
    }

    pub async fn peer_count(&self) -> usize {
        self.client.peer_count().await
    }

    pub async fn peers(&self) -> Vec<PeerId> {
        self.client.peers().await
    }

    pub async fn broadcast(&self, payload: Arc<[u8]>) -> usize {
        self.client.broadcast(payload).await
    }

    pub async fn broadcast_object(&self, object: Arc<[u8]>) -> usize {
        self.client.broadcast_object(object).await
    }

    /// Signs `payload` as a legacy simple or FEC overlay broadcast and sends it
    /// directly to every connected peer.
    ///
    /// This path does not use Plumtree state or repair. It is retained for
    /// external-message compatibility; use [`Self::broadcast_plumtree_simple`]
    /// or [`Self::broadcast_plumtree_fec`] for public-overlay payloads.
    /// The returned count is the number of successful session sends, not an
    /// application delivery guarantee.
    pub async fn broadcast_signed(
        &self,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        self.client.broadcast_signed(payload, flags).await
    }

    /// Queues one Plumtree simple broadcast for eager neighbours.
    ///
    /// The returned count is immediately queued payload frames, not a delivery
    /// guarantee. The background Plumtree pump performs the session sends.
    pub async fn broadcast_plumtree_simple(
        &self,
        id: PlumtreeBroadcastId,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        self.client
            .broadcast_plumtree_simple(id, payload, flags)
            .await
    }

    /// Encodes `payload` with RaptorQ and queues every Plumtree FEC part.
    ///
    /// The returned count is immediately queued payload frames, not a delivery
    /// guarantee. The background Plumtree pump performs the session sends.
    pub async fn broadcast_plumtree_fec(
        &self,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        self.client.broadcast_plumtree_fec(payload, flags).await
    }

    /// Returns the local Plumtree counters for this overlay.
    pub fn plumtree_stats(&self) -> PlumtreeStats {
        self.client.plumtree_stats()
    }

    pub fn shutdown(&self) {
        self.client.shutdown();
    }

    pub async fn shutdown_wait(&self) {
        self.client.shutdown_wait().await;
        let tasks = std::mem::take(&mut *self.tasks.lock().await);
        for task in tasks {
            let _ = task.await;
        }
    }
}
