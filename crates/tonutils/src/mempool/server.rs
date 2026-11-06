//! External-message server consuming one overlay client.

use std::sync::{Arc, atomic::Ordering};

use futures::Stream;

use crate::overlay::{OverlayClient, OverlayPacket};

use super::{
    MempoolConfig, MempoolError, MempoolEvent, MempoolMetrics, client::MempoolClient,
    client::MempoolCore, event_stream, unwrap_external_message_payload,
};

/// External-message ingest, deduplication, and event server.
///
/// The server subscribes to an already-built [`OverlayClient`]; it never
/// starts a transport, performs discovery, or publishes DHT records.
pub struct MempoolServer {
    client: MempoolClient,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for MempoolServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MempoolServer")
            .field("metrics", &self.client.metrics())
            .finish_non_exhaustive()
    }
}

impl MempoolServer {
    /// Creates the ingest server and returns it together with the shared
    /// client used for submissions.
    pub fn new(
        overlay: OverlayClient,
        config: MempoolConfig,
    ) -> Result<(Self, MempoolClient), MempoolError> {
        let core = Arc::new(MempoolCore::new(overlay, config)?);
        let client = MempoolClient::new(Arc::clone(&core));
        // Subscribe before spawning so packets and statuses emitted
        // immediately after startup cannot race the receiver tasks.
        let mut packet_receiver = core.overlay.subscribe_packets();
        let mut status_receiver = core.overlay.subscribe_statuses();
        let packet_core = Arc::clone(&core);
        let packets = tokio::spawn(async move {
            loop {
                match packet_receiver.recv().await {
                    Ok(OverlayPacket { payload, routing }) => {
                        if ingest_packet(&packet_core, payload, routing).await
                            == Err(MempoolError::QueueClosed)
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        let status_core = Arc::clone(&core);
        let statuses = tokio::spawn(async move {
            loop {
                match status_receiver.recv().await {
                    Ok(status) => {
                        if status_core
                            .event_tx
                            .send(MempoolEvent::PeerStatus(status))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok((
            Self {
                client: client.clone(),
                tasks: vec![packets, statuses],
            },
            client,
        ))
    }

    /// Returns the shared submission client.
    pub fn client(&self) -> MempoolClient {
        self.client.clone()
    }

    /// Returns the accept/dedup event stream.
    pub fn events(&self) -> impl Stream<Item = MempoolEvent> + use<> {
        event_stream(Arc::clone(&self.client.core.event_rx))
    }

    /// Runs `handler` for every event until the queue closes.
    pub async fn run_handler<H, F>(&self, handler: H)
    where
        H: FnMut(MempoolEvent) -> F,
        F: std::future::Future<Output = ()>,
    {
        self.client.run_handler(handler).await;
    }

    pub fn metrics(&self) -> MempoolMetrics {
        self.client.metrics()
    }

    /// Stops packet and status forwarding.
    pub fn shutdown(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }

    /// Stops forwarding and waits for both tasks to finish.
    pub async fn shutdown_wait(mut self) {
        self.shutdown();
        for task in self.tasks.drain(..) {
            let _ = task.await;
        }
    }
}

async fn ingest_packet(
    core: &Arc<MempoolCore>,
    payload: Arc<[u8]>,
    routing: crate::overlay::RoutingMetadata,
) -> Result<(), MempoolError> {
    core.overlay_packets.fetch_add(1, Ordering::Relaxed);
    core.queue_full_drops
        .store(core.overlay.ingest_drops(), Ordering::Relaxed);
    let size = payload.len();
    let prefix_len = size.min(8);
    let mut prefix = [0u8; 8];
    prefix[..prefix_len].copy_from_slice(&payload[..prefix_len]);
    let raw = unwrap_external_message_payload(payload.as_ref().to_vec());
    match core.accept(Arc::from(raw), routing, false).await {
        Ok(_) => Ok(()),
        Err(MempoolError::QueueClosed) => Err(MempoolError::QueueClosed),
        Err(error) => {
            core.invalid_warnings.fetch_add(1, Ordering::Relaxed);
            log::warn!(
                "dropping invalid overlay packet: size={size} first_bytes={} error={error}",
                hex::encode(&prefix[..prefix_len]),
            );
            Ok(())
        }
    }
}
