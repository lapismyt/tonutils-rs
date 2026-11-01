//! High-level overlay client handle.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::sync::{Mutex, broadcast};

use super::{
    DiscoveryStats, OverlayBroadcastPeer, OverlayError, OverlayMemberEndpoints, OverlayPacket,
    OverlayPeerPool, PeerId, PeerManager, PeerQualityRegistry, PeerStatus, PlumtreeBroadcastId,
    PlumtreeHandle, PlumtreeStats, ProtocolStats, SeedPeer, protocol_stats,
};

/// A cloneable handle that owns overlay transport, discovery, and broadcast
/// state for one node.
///
/// Packets and peer statuses are exposed as broadcast subscriptions so an
/// application protocol, such as [`crate::mempool::MempoolServer`], can consume
/// them without taking ownership of the overlay stack.
#[derive(Clone)]
pub struct OverlayClient {
    inner: Arc<Inner>,
}

struct Inner {
    manager: PeerManager,
    broadcast: Option<OverlayBroadcastPeer>,
    quality: Arc<PeerQualityRegistry>,
    packets: broadcast::Sender<OverlayPacket>,
    packet_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    plumtree: Option<PlumtreeHandle>,
    plumtree_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    member_endpoints: OverlayMemberEndpoints,
    known_peers: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    discovery_seeds: AtomicU64,
    discovery_results: AtomicU64,
    discovery_peers: AtomicU64,
}

pub(crate) struct OverlayClientInit {
    pub manager: PeerManager,
    pub broadcast: Option<OverlayBroadcastPeer>,
    pub quality: Arc<PeerQualityRegistry>,
    pub member_endpoints: OverlayMemberEndpoints,
    pub known_peers: Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    pub discovery: DiscoveryStats,
    pub packet_capacity: usize,
    pub plumtree: Option<PlumtreeHandle>,
}

impl OverlayClient {
    pub(crate) fn new(init: OverlayClientInit) -> Self {
        let OverlayClientInit {
            manager,
            broadcast,
            quality,
            member_endpoints,
            known_peers,
            discovery,
            packet_capacity,
            plumtree,
        } = init;
        let (packets, _) = broadcast::channel(packet_capacity);
        let task = spawn_packet_pump(&manager, packets.clone());
        let plumtree_task = plumtree.clone().map(|plumtree| {
            super::plumtree::spawn_plumtree_tasks(plumtree, manager.clone(), packets.clone())
        });
        Self {
            inner: Arc::new(Inner {
                manager,
                broadcast,
                quality,
                packets,
                packet_task: Mutex::new(Some(task)),
                plumtree,
                plumtree_task: Mutex::new(plumtree_task),
                member_endpoints,
                known_peers,
                discovery_seeds: AtomicU64::new(discovery.seeds),
                discovery_results: AtomicU64::new(discovery.discovered),
                discovery_peers: AtomicU64::new(discovery.peers),
            }),
        }
    }

    /// Subscribes to verified overlay packets.
    pub fn subscribe_packets(&self) -> broadcast::Receiver<OverlayPacket> {
        self.inner.packets.subscribe()
    }

    /// Subscribes to peer connection lifecycle changes.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<PeerStatus> {
        self.inner.manager.subscribe_statuses()
    }

    /// Returns the shared peer manager.
    pub fn manager(&self) -> PeerManager {
        self.inner.manager.clone()
    }

    /// Returns the delivery-quality registry used for send ordering and
    /// peer churn.
    pub fn quality(&self) -> Arc<PeerQualityRegistry> {
        Arc::clone(&self.inner.quality)
    }

    /// Returns the shared member endpoint registry.
    pub fn member_endpoints(&self) -> OverlayMemberEndpoints {
        Arc::clone(&self.inner.member_endpoints)
    }

    /// Returns the bootstrap snapshot used to resolve reconnect targets.
    pub async fn known_peers(&self) -> Vec<SeedPeer> {
        self.inner.known_peers.read().await.clone()
    }

    /// Returns the outcome of startup discovery.
    pub fn discovery_stats(&self) -> DiscoveryStats {
        DiscoveryStats {
            seeds: self.inner.discovery_seeds.load(Ordering::Relaxed),
            discovered: self.inner.discovery_results.load(Ordering::Relaxed),
            peers: self.inner.discovery_peers.load(Ordering::Relaxed),
        }
    }

    /// Returns process-wide overlay protocol counters.
    pub fn protocol_stats(&self) -> ProtocolStats {
        protocol_stats()
    }

    /// Returns the number of connected overlay sessions.
    pub async fn peer_count(&self) -> usize {
        self.inner.manager.peer_count().await
    }

    /// Returns the connected peer ids.
    pub async fn peers(&self) -> Vec<PeerId> {
        self.inner.manager.peers().await
    }

    /// Sends a payload wrapped in `overlay.unicast` to every connected peer.
    pub async fn broadcast(&self, payload: Arc<[u8]>) -> usize {
        self.inner.manager.broadcast(payload).await
    }

    /// Sends a raw overlay object to every connected peer.
    pub async fn broadcast_object(&self, object: Arc<[u8]>) -> usize {
        self.inner.manager.broadcast_object(object).await
    }

    /// Sends a raw overlay object to every peer, starting with `ranked`.
    pub async fn broadcast_object_in_order(&self, object: Arc<[u8]>, ranked: &[PeerId]) -> usize {
        self.inner
            .manager
            .broadcast_object_in_order(object, ranked)
            .await
    }

    /// Signs `payload` as a legacy generic simple or FEC overlay broadcast
    /// and fans it out to every connected peer.
    ///
    /// This is the external-message broadcast path retained for
    /// compatibility with [`crate::mempool::MempoolClient::send_external`];
    /// it does not use Plumtree state, repair, or eager-neighbour feedback.
    /// New public-overlay traffic should use
    /// [`Self::broadcast_plumtree_simple`] or [`Self::broadcast_plumtree_fec`].
    ///
    /// Returns the number of successful session sends, not an application
    /// delivery guarantee.
    pub async fn broadcast_signed(
        &self,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        let broadcast = self
            .inner
            .broadcast
            .as_ref()
            .ok_or(OverlayError::MissingLocalKey)?;
        broadcast.broadcast(payload.as_ref(), flags).await
    }

    /// Broadcasts `payload` as one Plumtree simple broadcast.
    ///
    /// Returns the number of payload frames immediately queued for eager
    /// neighbours. The background Plumtree pump performs the actual session
    /// sends, so this is not a delivery guarantee.
    pub async fn broadcast_plumtree_simple(
        &self,
        id: PlumtreeBroadcastId,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        self.plumtree_handle()?
            .broadcast_simple(id, payload.to_vec(), flags)
            .await
    }

    /// Encodes `payload` with RaptorQ and broadcasts every Plumtree FEC part.
    ///
    /// Returns the number of payload frames immediately queued for eager
    /// neighbours. The background Plumtree pump performs the actual session
    /// sends, so this is not a delivery guarantee.
    pub async fn broadcast_plumtree_fec(
        &self,
        payload: Arc<[u8]>,
        flags: i32,
    ) -> Result<usize, OverlayError> {
        self.plumtree_handle()?
            .broadcast_fec(payload.to_vec(), flags)
            .await
    }

    /// Returns this overlay's Plumtree counters.
    pub fn plumtree_stats(&self) -> PlumtreeStats {
        self.inner
            .plumtree
            .as_ref()
            .map_or_else(PlumtreeStats::default, PlumtreeHandle::stats)
    }

    fn plumtree_handle(&self) -> Result<PlumtreeHandle, OverlayError> {
        self.inner.plumtree.clone().ok_or_else(|| {
            OverlayError::PlumtreeUnavailable(
                "this overlay client has no Plumtree state".to_owned(),
            )
        })
    }

    /// Returns the packet pool's drop counter.
    pub fn ingest_drops(&self) -> u64 {
        self.inner.manager.pool().ingest_drops()
    }

    /// Signals every overlay task to stop.
    pub fn shutdown(&self) {
        self.inner.manager.shutdown();
    }

    /// Stops the manager and packet pump, then waits for their tasks.
    pub async fn shutdown_wait(&self) {
        self.inner.manager.shutdown_wait().await;
        if let Some(task) = self.inner.packet_task.lock().await.take() {
            let _ = task.await;
        }
        if let Some(task) = self.inner.plumtree_task.lock().await.take() {
            let _ = task.await;
        }
    }
}

fn spawn_packet_pump(
    manager: &PeerManager,
    packets: broadcast::Sender<OverlayPacket>,
) -> tokio::task::JoinHandle<()> {
    let pool: Arc<OverlayPeerPool> = manager.pool();
    let mut shutdown = manager.subscribe_shutdown();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                packet = pool.next_packet() => {
                    let Some(packet) = packet else { break };
                    let _ = packets.send(packet);
                }
            }
        }
    })
}
