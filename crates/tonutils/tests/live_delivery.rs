#![cfg(all(feature = "mempool", feature = "overlay-config"))]

//! Mandatory public-overlay Plumtree delivery gate.

mod live_common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use live_common::{configured_overlay_id, elapsed_millis, load_global_config};
use tonutils::{
    adnl::AdnlKeyPair,
    mempool::{MempoolConfig, MempoolEvent, MempoolMetrics, MempoolServer},
    overlay::{
        DiscoveryStats, OverlayConfig, OverlayId, OverlayNodeBuilder, OverlayPacket, OverlayServer,
        PeerStatus, PlumtreeConfig, PlumtreeStats, ProtocolStats,
    },
};

const DELIVERY_ATTEMPTS: usize = 3;
const DELIVERY_WINDOW: Duration = Duration::from_secs(120);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct DeliveryEvidence {
    elapsed_ms: u128,
    packet_bytes: usize,
    packet_count: u64,
    stats: PlumtreeStats,
    peers: usize,
    mempool: MempoolMetrics,
    external_events: u64,
    all_events: u64,
    discovery: DiscoveryStats,
    protocol: ProtocolStats,
}

#[tokio::test]
#[ignore = "requires live public mainnet overlay traffic"]
async fn mainnet_public_plumtree_receives_payload() -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let config_json = load_global_config("TON_GLOBAL_CONFIG_JSON")?;
    let overlay = configured_overlay_id()?;
    let max_peers = std::env::var("TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256u32);
    let mut failures = Vec::new();

    for attempt in 1..=DELIVERY_ATTEMPTS {
        eprintln!(
            "live delivery attempt={attempt} phase=start overlay={overlay} window_s={} max_peers={max_peers}",
            DELIVERY_WINDOW.as_secs(),
        );
        match run_attempt(attempt, &config_json, overlay, max_peers).await {
            Ok(evidence) => {
                eprintln!(
                    "live delivery attempt={attempt} result=delivered elapsed_ms={} packet_bytes={} packets={} parts_received={} completed_broadcasts={} peers={} mempool={:?} external_events={} all_events={} discovery={:?} protocol={:?}",
                    evidence.elapsed_ms,
                    evidence.packet_bytes,
                    evidence.packet_count,
                    evidence.stats.parts_received,
                    evidence.stats.completed_broadcasts,
                    evidence.peers,
                    evidence.mempool,
                    evidence.external_events,
                    evidence.all_events,
                    evidence.discovery,
                    evidence.protocol,
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!("live delivery attempt={attempt} result=failed error={error:#}");
                failures.push(format!("attempt {attempt}: {error:#}"));
            }
        }
    }

    bail!(
        "no public Plumtree payload within {DELIVERY_ATTEMPTS} attempts of {}s: {}",
        DELIVERY_WINDOW.as_secs(),
        failures.join(" | "),
    )
}

async fn run_attempt(
    attempt: usize,
    config_json: &str,
    overlay: OverlayId,
    max_peers: u32,
) -> Result<DeliveryEvidence> {
    let local_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let started = Instant::now();
    let server = OverlayNodeBuilder::new()
        .testnet(false)
        .global_config_json(config_json)
        .download_config(false)
        .overlay_id(overlay)
        .dht_overlay_key(overlay.as_bytes())
        .bootstrap_timeout(Duration::from_secs(30))
        .discovery_timeout(Duration::from_secs(30))
        .overlay_config(OverlayConfig {
            growth_interval: Duration::from_secs(3),
            ..OverlayConfig::default()
        })
        .overlay_max_peers(max_peers)
        .plumtree(PlumtreeConfig::default())
        .native_udp(
            "0.0.0.0:0".parse().expect("wildcard address must parse"),
            local_key,
            None,
        )
        .start_server()
        .await
        .with_context(|| format!("attempt {attempt} overlay bootstrap failed"))?;
    let peers = server.peer_count().await;
    if peers == 0 {
        server.shutdown_wait().await;
        bail!("attempt {attempt} bootstrap registered no overlay peers");
    }
    eprintln!(
        "live delivery attempt={attempt} phase=bootstrap elapsed_ms={} peers={peers} discovery={:?}",
        elapsed_millis(started),
        server.discovery_stats(),
    );

    let packets = server.subscribe_packets();
    let (mempool, _client) = match MempoolServer::new(server.client(), MempoolConfig::default()) {
        Ok(value) => value,
        Err(error) => {
            server.shutdown_wait().await;
            return Err(error).context("MempoolServer construction failed");
        }
    };
    let external_events = Arc::new(AtomicU64::new(0));
    let all_events = Arc::new(AtomicU64::new(0));
    let event_task = spawn_event_logger(
        mempool.events(),
        Arc::clone(&external_events),
        Arc::clone(&all_events),
        attempt,
    );

    let result = wait_for_payload(
        attempt,
        started,
        &server,
        &mempool,
        packets,
        Arc::clone(&external_events),
        Arc::clone(&all_events),
    )
    .await;
    event_task.abort();
    let _ = event_task.await;
    mempool.shutdown_wait().await;
    server.shutdown_wait().await;
    result
}

fn spawn_event_logger<S>(
    events: S,
    external_events: Arc<AtomicU64>,
    all_events: Arc<AtomicU64>,
    attempt: usize,
) -> tokio::task::JoinHandle<()>
where
    S: futures::Stream<Item = MempoolEvent> + Send + 'static,
{
    tokio::spawn(async move {
        let mut events = Box::pin(events);
        while let Some(event) = events.next().await {
            all_events.fetch_add(1, Ordering::Relaxed);
            match event {
                MempoolEvent::ExternalMessage {
                    hash,
                    raw_boc,
                    destination,
                    routing,
                    timestamp,
                } => {
                    external_events.fetch_add(1, Ordering::Relaxed);
                    let destination = destination.map_or_else(|| "none".to_owned(), hex::encode);
                    eprintln!(
                        "live delivery attempt={attempt} mempool event=external_message hash={} bytes={} destination={destination} peer={} overlay={} hop={} timestamp_unix_ms={}",
                        hex::encode(hash),
                        raw_boc.len(),
                        hex::encode(routing.peer.as_bytes()),
                        routing.overlay,
                        routing.hop,
                        unix_millis(timestamp),
                    );
                }
                MempoolEvent::Included {
                    hash,
                    block,
                    transaction,
                } => eprintln!(
                    "live delivery attempt={attempt} mempool event=included hash={} block_bytes={} transaction_bytes={:?}",
                    hex::encode(hash),
                    block.len(),
                    transaction.as_ref().map(|transaction| transaction.len()),
                ),
                MempoolEvent::PeerStatus(status) => {
                    eprintln!(
                        "live delivery attempt={attempt} mempool event=peer_status status={}",
                        peer_status_label(&status),
                    );
                }
            }
        }
    })
}

async fn wait_for_payload(
    attempt: usize,
    started: Instant,
    server: &OverlayServer,
    mempool: &MempoolServer,
    mut packets: tokio::sync::broadcast::Receiver<OverlayPacket>,
    external_events: Arc<AtomicU64>,
    all_events: Arc<AtomicU64>,
) -> Result<DeliveryEvidence> {
    let deadline = tokio::time::Instant::now() + DELIVERY_WINDOW;
    let mut progress = tokio::time::interval(PROGRESS_INTERVAL);
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut packet_count = 0u64;
    let mut packet_bytes = 0usize;

    loop {
        let stats = server.plumtree_stats();
        if stats.completed_broadcasts > 0 && packet_count > 0 {
            return Ok(DeliveryEvidence {
                elapsed_ms: elapsed_millis(started),
                packet_bytes,
                packet_count,
                stats,
                peers: server.peer_count().await,
                mempool: mempool.metrics(),
                external_events: external_events.load(Ordering::Relaxed),
                all_events: all_events.load(Ordering::Relaxed),
                discovery: server.discovery_stats(),
                protocol: server.protocol_stats(),
            });
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            let peers = server.peer_count().await;
            bail!(
                "attempt {attempt} timed out after {}s waiting for parts_received>0, completed_broadcasts>0, and a subscribe_packets event: parts_received={} completed_broadcasts={} packets={packet_count} peers={peers} mempool={:?} external_events={} all_events={} discovery={:?} protocol={:?}",
                DELIVERY_WINDOW.as_secs(),
                stats.parts_received,
                stats.completed_broadcasts,
                mempool.metrics(),
                external_events.load(Ordering::Relaxed),
                all_events.load(Ordering::Relaxed),
                server.discovery_stats(),
                server.protocol_stats(),
            );
        }

        tokio::select! {
            packet = packets.recv() => {
                match packet {
                    Ok(packet) => {
                        packet_count += 1;
                        packet_bytes = packet.payload.len();
                        let stats = server.plumtree_stats();
                        eprintln!(
                            "live delivery attempt={attempt} phase=packet bytes={} peer={} overlay={} hop={} received_unix_ms={} parts_received={} completed_broadcasts={}",
                            packet.payload.len(),
                            hex::encode(packet.routing.peer.as_bytes()),
                            packet.routing.overlay,
                            packet.routing.hop,
                            packet.routing.received_unix_millis(),
                            stats.parts_received,
                            stats.completed_broadcasts,
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!(
                            "live delivery attempt={attempt} phase=packet result=lagged skipped={skipped}"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        bail!("attempt {attempt} overlay packet subscription closed");
                    }
                }
            }
            _ = progress.tick() => {
                eprintln!(
                    "live delivery attempt={attempt} phase=waiting elapsed_ms={} parts_received={} completed_broadcasts={} packets={packet_count} peers={} mempool={:?} external_events={} all_events={}",
                    elapsed_millis(started),
                    stats.parts_received,
                    stats.completed_broadcasts,
                    server.peer_count().await,
                    mempool.metrics(),
                    external_events.load(Ordering::Relaxed),
                    all_events.load(Ordering::Relaxed),
                );
            }
            () = tokio::time::sleep_until(deadline) => {}
        }
    }
}

fn peer_status_label(status: &PeerStatus) -> String {
    match status {
        PeerStatus::Connected { peer } => {
            format!("connected peer={}", hex::encode(peer.as_bytes()))
        }
        PeerStatus::Disconnected { peer } => {
            format!("disconnected peer={}", hex::encode(peer.as_bytes()))
        }
        PeerStatus::Failed { peer } => format!("failed peer={}", hex::encode(peer.as_bytes())),
        PeerStatus::Reconnecting { peer, attempt } => format!(
            "reconnecting peer={} attempt={attempt}",
            hex::encode(peer.as_bytes())
        ),
    }
}

fn unix_millis(timestamp: SystemTime) -> u128 {
    timestamp
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
