//! Background send and maintenance pumps for shared Plumtree state.

use std::{sync::Arc, time::Duration};

use tokio::sync::broadcast;

use crate::overlay::{OverlayPacket, PeerManager, RoutingMetadata};

use super::{Action, PlumtreeHandle, RepairRequest};

/// Upstream bounds one repair query by the pending-feedback TTL.
const REPAIR_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Maintenance cadence: repairs marked due by `IHave` start within one
/// repair timeout, while garbage collection and feedback expiry stay cheap.
const TICK_INTERVAL: Duration = Duration::from_millis(50);
/// How often the known-neighbour set is refreshed from the manager.
const PEER_SYNC_INTERVAL: Duration = Duration::from_secs(5);

/// Spawns the action pump that drives one overlay's Plumtree state.
///
/// The returned task owns the handle's action receiver: every `Send`
/// frame leaves through [`PeerManager::send_object_to`], every repair
/// request becomes an `overlay.repairPlumtreePart` query, and a
/// successfully decoded repair response is published on `packets` as if
/// it had arrived as a push.
pub(crate) fn spawn_plumtree_tasks(
    handle: PlumtreeHandle,
    manager: PeerManager,
    packets: broadcast::Sender<OverlayPacket>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let Some(mut actions) = handle.take_action_receiver() else {
            return;
        };
        let overlay = manager.overlay();
        let mut shutdown = manager.subscribe_shutdown();
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut peer_sync = tokio::time::interval(PEER_SYNC_INTERVAL);
        peer_sync.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        handle.update_peers(manager.peers().await).await;
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = ticker.tick() => {
                    let now = std::time::Instant::now();
                    handle.tick(now).await;
                    for request in handle.due_repairs(now).await {
                        spawn_repair(&handle, &manager, &packets, overlay, request);
                    }
                }
                _ = peer_sync.tick() => {
                    handle.update_peers(manager.peers().await).await;
                }
                action = actions.recv() => {
                    match action {
                        Some(Action::Send { peer, wire }) => {
                            let sent = manager
                                .send_object_to(peer, Arc::from(wire))
                                .await;
                            if !sent {
                                log::debug!(
                                    "Plumtree frame dropped for peer without a session: peer={peer:?}"
                                );
                            }
                        }
                        Some(Action::Repair(request)) => {
                            spawn_repair(&handle, &manager, &packets, overlay, request);
                        }
                        None => break,
                    }
                }
            }
        }
    })
}

fn spawn_repair(
    handle: &PlumtreeHandle,
    manager: &PeerManager,
    packets: &broadcast::Sender<OverlayPacket>,
    overlay: crate::overlay::OverlayId,
    request: RepairRequest,
) {
    let query = match handle.repair_query(request.key) {
        Ok(query) => query,
        Err(error) => {
            log::debug!(
                "Plumtree repair query could not be built: peer={:?} error={error}",
                request.peer
            );
            let handle = handle.clone();
            tokio::spawn(async move {
                handle.finish_repair(request.key).await;
            });
            return;
        }
    };
    let handle = handle.clone();
    let manager = manager.clone();
    let packets = packets.clone();
    tokio::spawn(async move {
        let answer = manager
            .send_query_to(request.peer, Arc::from(query), REPAIR_QUERY_TIMEOUT)
            .await;
        handle.finish_repair(request.key).await;
        let Ok(answer) = answer else {
            return;
        };
        match handle
            .process_repair_response(request.peer, request.key, &answer)
            .await
        {
            Ok(Some(payload)) => {
                let _ = packets.send(OverlayPacket {
                    payload: Arc::from(payload),
                    routing: RoutingMetadata::new(overlay, request.peer),
                });
            }
            Ok(None) => {}
            Err(error) => {
                log::debug!(
                    "Plumtree repair response rejected: peer={:?} error={error}",
                    request.peer
                );
            }
        }
    });
}
