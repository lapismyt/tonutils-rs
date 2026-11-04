#![cfg(all(feature = "mempool", feature = "overlay-config"))]

//! Mandatory public-overlay delivery gate.
//!
//! The public mainnet basechain overlay currently carries legacy
//! `overlay.broadcast` / `overlay.broadcastFec` frames for external messages.
//! The gate therefore requires a real public payload to reach
//! `subscribe_packets()` (legacy/FEC accepted) and keeps Plumtree completion
//! covered by the deterministic two-node integration tests.

mod live_common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use live_common::{configured_overlay_id, elapsed_millis, load_global_config};
use tonutils::{
    adnl::AdnlKeyPair,
    mempool::{MempoolConfig, MempoolEvent, MempoolMetrics, MempoolServer},
    overlay::{
        DiscoveryStats, OverlayConfig, OverlayId, OverlayNodeBuilder, OverlayPacket, OverlayServer,
        PeerStatus, PlumtreeConfig, PlumtreeStats, ProtocolStats,
        protocol_stats as global_protocol_stats,
    },
};

const DELIVERY_ATTEMPTS: usize = 3;
/// Public external-message traffic was observed about 220 seconds after
/// bootstrap in the November 2026 CI run, so a 180-second window could miss
/// the first public payload entirely. Each attempt verifies the same strict
/// delivery conditions; the wider window only gives public traffic time to
/// reach the freshly bootstrapped node.
const DELIVERY_WINDOW: Duration = Duration::from_secs(300);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

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
    custom_messages_delta: u64,
    framing: &'static str,
}

#[derive(Clone, Copy, Default)]
struct PacketsSeen {
    count: u64,
    last_bytes: usize,
}

#[derive(Clone, Copy)]
struct Observation {
    stats: PlumtreeStats,
    protocol: ProtocolStats,
    mempool: MempoolMetrics,
    custom_messages_delta: u64,
}

/// Per-attempt state shared by the wait loop and its evidence helpers.
struct DeliveryRun<'a> {
    attempt: usize,
    started: Instant,
    server: &'a OverlayServer,
    mempool: &'a MempoolServer,
    packets: tokio::sync::broadcast::Receiver<OverlayPacket>,
    protocol_start: ProtocolStats,
    external_events: Arc<AtomicU64>,
    all_events: Arc<AtomicU64>,
}

impl DeliveryRun<'_> {
    fn observe(&self) -> Observation {
        let protocol = self.server.protocol_stats();
        Observation {
            stats: self.server.plumtree_stats(),
            protocol,
            mempool: self.mempool.metrics(),
            custom_messages_delta: protocol
                .custom_messages
                .saturating_sub(self.protocol_start.custom_messages),
        }
    }

    async fn evidence(&self, seen: PacketsSeen, observation: &Observation) -> DeliveryEvidence {
        let framing = if observation.stats.completed_broadcasts > 0 {
            "plumtree"
        } else {
            "legacy"
        };
        DeliveryEvidence {
            elapsed_ms: elapsed_millis(self.started),
            packet_bytes: seen.last_bytes,
            packet_count: seen.count,
            stats: observation.stats,
            peers: self.server.peer_count().await,
            mempool: observation.mempool,
            external_events: self.external_events.load(Ordering::Relaxed),
            all_events: self.all_events.load(Ordering::Relaxed),
            discovery: self.server.discovery_stats(),
            protocol: observation.protocol,
            custom_messages_delta: observation.custom_messages_delta,
            framing,
        }
    }

    async fn timeout_error(&self, seen: PacketsSeen, observation: &Observation) -> anyhow::Error {
        let peers = self.server.peer_count().await;
        anyhow!(
            "attempt {} timed out after {}s waiting for a delivered public payload: packets={} peers={peers} custom_messages_delta={} payload_duplicates_delta={} fec_decoded_delta={} parts_received={} completed_broadcasts={} ingest_drops={} mempool={:?} external_events={} all_events={} discovery={:?} protocol={:?}",
            self.attempt,
            DELIVERY_WINDOW.as_secs(),
            seen.count,
            observation.custom_messages_delta,
            observation
                .protocol
                .payload_duplicates
                .saturating_sub(self.protocol_start.payload_duplicates),
            observation
                .protocol
                .fec_decoded
                .saturating_sub(self.protocol_start.fec_decoded),
            observation.stats.parts_received,
            observation.stats.completed_broadcasts,
            self.server.client().ingest_drops(),
            observation.mempool,
            self.external_events.load(Ordering::Relaxed),
            self.all_events.load(Ordering::Relaxed),
            self.server.discovery_stats(),
            observation.protocol,
        )
    }
}

#[tokio::test]
#[ignore = "requires live public mainnet overlay traffic"]
async fn mainnet_public_overlay_delivers_payload() -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let config_json = load_global_config("TON_GLOBAL_CONFIG_JSON")?;
    let overlay = configured_overlay_id()?;
    let max_peers = std::env::var("TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64u32);
    let mut failures = Vec::new();

    for attempt in 1..=DELIVERY_ATTEMPTS {
        eprintln!(
            "live delivery attempt={attempt} phase=start overlay={overlay} window_s={} max_peers={max_peers}",
            DELIVERY_WINDOW.as_secs(),
        );
        match run_attempt(attempt, &config_json, overlay, max_peers).await {
            Ok(evidence) => {
                eprintln!(
                    "live delivery attempt={attempt} result=delivered elapsed_ms={} framing={} packet_bytes={} packets={} parts_received={} completed_broadcasts={} custom_messages_delta={} peers={} mempool={:?} external_events={} all_events={} discovery={:?} protocol={:?}",
                    evidence.elapsed_ms,
                    evidence.framing,
                    evidence.packet_bytes,
                    evidence.packet_count,
                    evidence.stats.parts_received,
                    evidence.stats.completed_broadcasts,
                    evidence.custom_messages_delta,
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
        "no public overlay payload within {DELIVERY_ATTEMPTS} attempts of {}s: {}",
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
    // Snapshot the process-wide protocol counters so every attempt reports
    // only the broadcasts it observed itself.
    let protocol_start = global_protocol_stats();
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
        shutdown_server(attempt, &server).await;
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
            shutdown_server(attempt, &server).await;
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

    let mut run = DeliveryRun {
        attempt,
        started,
        server: &server,
        mempool: &mempool,
        packets,
        protocol_start,
        external_events: Arc::clone(&external_events),
        all_events: Arc::clone(&all_events),
    };
    let result = wait_for_payload(&mut run).await;
    event_task.abort();
    let _ = event_task.await;
    mempool.shutdown_wait().await;
    shutdown_server(attempt, &server).await;
    result
}

async fn shutdown_server(attempt: usize, server: &OverlayServer) {
    server.shutdown();
    if tokio::time::timeout(SHUTDOWN_GRACE, server.shutdown_wait())
        .await
        .is_err()
    {
        eprintln!(
            "live delivery attempt={attempt} phase=shutdown result=timeout grace_s={}",
            SHUTDOWN_GRACE.as_secs(),
        );
    }
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

async fn wait_for_payload(run: &mut DeliveryRun<'_>) -> Result<DeliveryEvidence> {
    let deadline = tokio::time::Instant::now() + DELIVERY_WINDOW;
    let mut progress = tokio::time::interval(PROGRESS_INTERVAL);
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut seen = PacketsSeen::default();

    // Copies used inside `select!` so the packet future's mutable borrow of
    // `run.packets` does not overlap borrows of the logging fields.
    let attempt = run.attempt;
    let started = run.started;
    let server = run.server;
    let mempool = run.mempool;
    let protocol_start = run.protocol_start;
    let external_events = Arc::clone(&run.external_events);
    let all_events = Arc::clone(&run.all_events);

    loop {
        let observation = run.observe();
        if seen.count > 0
            && observation.mempool.overlay_packets > 0
            && (observation.custom_messages_delta > 0 || observation.stats.completed_broadcasts > 0)
        {
            return Ok(run.evidence(seen, &observation).await);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(run.timeout_error(seen, &observation).await);
        }

        tokio::select! {
            packet = run.packets.recv() => match packet {
                Ok(packet) => {
                    seen.count += 1;
                    seen.last_bytes = packet.payload.len();
                    log_packet(attempt, server, mempool, &protocol_start, &packet).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    eprintln!(
                        "live delivery attempt={attempt} phase=packet result=lagged skipped={skipped}"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    bail!("attempt {attempt} overlay packet subscription closed");
                }
            },
            _ = progress.tick() => {
                let peers = server.peer_count().await;
                eprintln!(
                    "live delivery attempt={attempt} phase=waiting elapsed_ms={} packets={} peers={peers} custom_messages_delta={} parts_received={} completed_broadcasts={} plumtree_parts_received={} mempool={:?} external_events={} all_events={}",
                    elapsed_millis(started),
                    seen.count,
                    observation.custom_messages_delta,
                    observation.stats.parts_received,
                    observation.stats.completed_broadcasts,
                    observation
                        .protocol
                        .plumtree_parts_received
                        .saturating_sub(protocol_start.plumtree_parts_received),
                    observation.mempool,
                    external_events.load(Ordering::Relaxed),
                    all_events.load(Ordering::Relaxed),
                );
            }
            () = tokio::time::sleep_until(deadline) => {}
        }
    }
}

async fn log_packet(
    attempt: usize,
    server: &OverlayServer,
    mempool: &MempoolServer,
    protocol_start: &ProtocolStats,
    packet: &OverlayPacket,
) {
    let stats = server.plumtree_stats();
    let protocol = server.protocol_stats();
    let custom_messages_delta = protocol
        .custom_messages
        .saturating_sub(protocol_start.custom_messages);
    let routing_peer_registered = server.peers().await.contains(&packet.routing.peer);
    let prefix_len = packet.payload.len().min(8);
    eprintln!(
        "live delivery attempt={attempt} phase=packet bytes={} prefix={} peer={} overlay={} hop={} received_unix_ms={} routing_peer_registered={routing_peer_registered} custom_messages_delta={custom_messages_delta} plumtree_parts_received={} completed_broadcasts={} mempool_overlay_packets={}",
        packet.payload.len(),
        hex::encode(&packet.payload[..prefix_len]),
        hex::encode(packet.routing.peer.as_bytes()),
        packet.routing.overlay,
        packet.routing.hop,
        packet.routing.received_unix_millis(),
        stats.parts_received,
        stats.completed_broadcasts,
        mempool.metrics().overlay_packets,
    );
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
