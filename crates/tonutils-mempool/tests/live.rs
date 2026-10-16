use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use futures::future::join_all;
use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_mempool::{
    MempoolConfig, MempoolEvent, OverlayMemberEndpoints, PeerQualityConfig, known_member_seeds,
    udp_overlay_lookup, udp_overlay_lookup_with_endpoints,
};
use tonutils_network_config::extract_dht_addresses;
use tonutils_overlay::{OverlayId, PeerId, PeerStatus, SeedPeer};
use tonutils_tl::Int256;
use tonutils_tl::tl::adnl::Message as AdnlMessage;
use tonutils_tl::tl::network::{DhtMessage, DhtNodesBoxed, PacketContents};

// Shared environment parsing and soft-skip assertion for the
// live tests in this directory; re-exported so call sites read
// unchanged.  The mainnet QUIC discovery test and its Version
// Negotiation probe helpers moved to `live_quic.rs`.
mod common;
pub use common::*;

fn configured_seed(variable: &str) -> SocketAddr {
    let value = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    value
        .parse()
        .unwrap_or_else(|_| panic!("{variable} must contain an IP:port seed"))
}

#[tokio::test]
#[ignore = "requires a configured mainnet seed and live network access"]
async fn mainnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_MAINNET_SEED");
}

#[tokio::test]
#[ignore = "requires a configured testnet seed and live network access"]
async fn testnet_seed_configuration_is_valid() {
    let _ = configured_seed("TON_MEMPOOL_TESTNET_SEED");
}

#[tokio::test]
#[ignore = "requires live network access"]
async fn configured_global_config_can_start_scanner() {
    let json = std::env::var("TON_GLOBAL_CONFIG_JSON")
        .expect("set TON_GLOBAL_CONFIG_JSON to run this ignored live test");
    let testnet = std::env::var("TON_NETWORK").as_deref() == Ok("testnet");
    let result = MempoolScannerBuilder::new()
        .testnet(testnet)
        .global_config_json(json)
        .download_config(false)
        .start()
        .await;
    let (_, manager, _) = result.expect("configured global config must yield bootstrap peers");
    manager.shutdown();
}

async fn probe_dht_from_config(variable: &str, allow_unavailable: bool) {
    let json = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to run this ignored live test"));
    let candidates =
        extract_dht_addresses(&json).expect("global config must contain DHT static nodes");
    let local = KeyPair::generate(&mut rand::rngs::OsRng);
    let mut last_error = None;
    for candidate in candidates.into_iter().take(16) {
        let Some(public_key) = candidate.public_key else {
            continue;
        };
        let Some(remote_public) = PublicKey::from_bytes(public_key) else {
            continue;
        };
        let session = match tokio::time::timeout(
            Duration::from_secs(5),
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().unwrap(),
                candidate.address,
                local,
                remote_public,
            ),
        )
        .await
        {
            Err(_timeout) => {
                last_error = Some(format!("connect timeout for {}", candidate.address));
                continue;
            }
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                continue;
            }
            Ok(Ok(session)) => session,
        };
        match session
            .dht_find_node(Int256::random(), 8, Duration::from_secs(5))
            .await
        {
            Ok(nodes) if !nodes.nodes.is_empty() => return,
            Ok(_) => {
                log::debug!("DHT probe {} returned no verified nodes", candidate.address);
                last_error = Some("empty verified DHT node response".into());
            }
            Err(error) => {
                log::debug!("DHT probe {} failed: {error}", candidate.address);
                last_error = Some(error.to_string());
            }
        }
    }
    let error = last_error.unwrap_or_else(|| "no usable peers".into());
    if allow_unavailable {
        log::debug!("skipping unavailable DHT probe: {error}");
    } else {
        panic!("no configured DHT peer answered: {error}");
    }
}

#[tokio::test]
#[ignore = "requires live mainnet QUIC access"]
async fn mainnet_dht_seed_answers_find_node() {
    let allow_unavailable = std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").is_ok();
    probe_dht_from_config("TON_GLOBAL_CONFIG_JSON", allow_unavailable).await;
}

#[tokio::test]
#[ignore = "requires live testnet QUIC access"]
async fn testnet_dht_seed_answers_find_node() {
    probe_dht_from_config("TON_TESTNET_GLOBAL_CONFIG_JSON", true).await;
}

/// Sends a real `dht.findNode` over ADNL/UDP to a mainnet config seed and
/// records the raw boxed `dht.nodes` answer bytes as the cross-SDK live
/// capture fixture `fixtures/cross_sdk/live_dht_answers.json`.
///
/// The fixture is committed and consumed offline by the tonutils-tl
/// `cross_sdk` tests (roundtrip-only: live answers have no canonical input
/// fields). Set `TON_CAPTURE_CROSS_SDK=1` together with
/// `TON_GLOBAL_CONFIG_JSON` to (re)capture it; without the variable the test
/// succeeds without touching the fixture so plain `--ignored` runs do not
/// overwrite committed evidence.
#[tokio::test]
#[ignore = "requires live mainnet DHT access to capture cross-SDK fixtures"]
async fn capture_cross_sdk_dht_answer_from_mainnet() {
    if std::env::var("TON_CAPTURE_CROSS_SDK").is_err() {
        eprintln!(
            "set TON_CAPTURE_CROSS_SDK=1 to (re)capture \
             fixtures/cross_sdk/live_dht_answers.json; skipping"
        );
        return;
    }
    let json = std::env::var("TON_GLOBAL_CONFIG_JSON")
        .expect("set TON_GLOBAL_CONFIG_JSON to run this ignored live test");
    let candidates =
        extract_dht_addresses(&json).expect("global config must contain DHT static nodes");
    let local = KeyPair::generate(&mut rand::rngs::OsRng);
    let mut last_error = None;
    for candidate in candidates.into_iter().take(16) {
        let Some(public_key) = candidate.public_key else {
            continue;
        };
        let Some(remote_public) = PublicKey::from_bytes(public_key) else {
            continue;
        };
        let mut session = match tokio::time::timeout(
            Duration::from_secs(5),
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().unwrap(),
                candidate.address,
                local,
                remote_public,
            ),
        )
        .await
        {
            Err(_timeout) => {
                last_error = Some(format!("connect timeout for {}", candidate.address));
                continue;
            }
            Ok(Err(error)) => {
                last_error = Some(error.to_string());
                continue;
            }
            Ok(Ok(session)) => session,
        };
        match capture_find_node_answer(&mut session).await {
            Ok(answer) => {
                let nodes: DhtNodesBoxed = tl_proto::deserialize(&answer)
                    .unwrap_or_else(|error| panic!("captured answer must parse: {error:?}"));
                assert!(
                    !nodes.nodes.is_empty(),
                    "captured dht.nodes answer from {} must not be empty",
                    candidate.address
                );
                write_live_fixture(&candidate.address.to_string(), &answer);
                return;
            }
            Err(error) => {
                last_error = Some(format!("{}: {error}", candidate.address));
            }
        }
    }
    let error = last_error.unwrap_or_else(|| "no usable DHT seed".into());
    panic!("no configured DHT seed answered findNode: {error}");
}

/// Performs the same `dht.findNode` exchange as `AdnlUdpSession::dht_find_node`
/// but returns the raw `AdnlMessage::Answer` payload bytes before any
/// parsing, so the wire bytes can be recorded verbatim.
async fn capture_find_node_answer(session: &mut AdnlUdpSession) -> Result<Vec<u8>, String> {
    let query_id = Int256::random();
    let query = tl_proto::serialize(DhtMessage::FindNode {
        key: Int256::random(),
        k: 8,
    });
    session
        .send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
            }),
            messages: None,
            address: None,
            priority_address: None,
            seqno: None,
            confirm_seqno: None,
            recv_addr_list_version: None,
            recv_priority_addr_list_version: None,
            reinit_date: None,
            dst_reinit_date: None,
            signature: None,
            rand2: vec![0; 7],
        })
        .await
        .map_err(|error| format!("send findNode: {error}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("timeout waiting for findNode answer".into());
        }
        let packet = session
            .recv_timeout(remaining)
            .await
            .map_err(|error| format!("recv: {error}"))?;
        let messages = packet
            .message
            .into_iter()
            .chain(packet.messages.into_iter().flatten());
        for message in messages {
            if let AdnlMessage::Answer {
                query_id: id,
                answer,
            } = message
                && id == query_id
            {
                return Ok(answer);
            }
        }
    }
}

fn write_live_fixture(peer: &str, answer: &[u8]) {
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_secs();
    let fixture = serde_json::json!({
        "schema_revision": "upstream:3d478cbde854be03a18ab2a59f8fc3c565cf7d14",
        "cases": [{
            "name": "live-dht-nodes-mainnet",
            "constructor": "dht.nodes",
            "references": [],
            "source": "live_capture",
            "captured_at_unix": captured_at,
            "peer": peer,
            "query": "dht.findNode k=8 with a random key over ADNL/UDP",
            "raw_hex": hex::encode(answer),
        }],
    });
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cross_sdk/live_dht_answers.json"
    );
    let mut text = serde_json::to_string_pretty(&fixture).expect("fixture must serialize");
    text.push('\n');
    std::fs::write(path, text).unwrap_or_else(|error| panic!("write {path}: {error}"));
    eprintln!("wrote {path} ({} bytes)", answer.len());
}

/// Delivers one valid external message over the live mainnet
/// mempool overlay, then proves non-seed connectivity: at least
/// one peer outside the configured seed list must complete an
/// authenticated overlay session, and a direct ADNL/UDP session
/// to a non-seed registry member must answer
/// `overlay.getRandomPeers`.
///
/// The seeds are `dht-server` deployments that do not run overlay
/// sessions, so every Connected peer and every registry member
/// comes from discovery or peer growth - never from the seed list.
#[tokio::test]
#[ignore = "requires a configured live overlay seed and external-message traffic"]
async fn configured_seed_delivers_valid_external_message() {
    let _ = pretty_env_logger::try_init();
    let seeds = configured_live_seeds();
    assert!(
        !seeds.is_empty(),
        "live seed phase failed: no seeds were configured"
    );
    assert!(
        seeds.iter().all(SeedPeer::is_valid),
        "live seed phase failed: every seed must contain a nonzero Ed25519 key and usable IP:port"
    );
    eprintln!("live mempool phase=seed validated={}", seeds.len());
    let overlay_bytes = configured_bytes("TON_MEMPOOL_LIVE_OVERLAY_ID")
        .expect("TON_MEMPOOL_LIVE_OVERLAY_ID must be set to 32-byte hex");
    let timeout = std::env::var("TON_MEMPOOL_LIVE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(120);
    // Peer growth is the only source of short-lived lookup sockets after
    // bootstrap, so setting this to 0 turns it off and makes a run isolatable
    // when a live failure has to be attributed to growth or to reachability.
    let overlay_max_peers = std::env::var("TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64);
    let local_key = KeyPair::generate(&mut rand::rngs::OsRng);
    let overlay = OverlayId::from_bytes(overlay_bytes);
    let dht_overlay_key: [u8; 32] = overlay_bytes;
    let bootstrap_started = tokio::time::Instant::now();
    // Shared with the builder so the UDP discovery, peer-growth and
    // session factory all record into the registry this test reads
    // after bootstrap: it holds every runtime-resolved member, which
    // is the non-seed peer set the connectivity checks below dial.
    let member_endpoints = OverlayMemberEndpoints::default();
    let (scanner, manager, stream) = MempoolScannerBuilder::new()
        .download_config(false)
        .overlay_id(overlay)
        .config(MempoolConfig {
            growth_interval: Duration::from_secs(3),
            ..MempoolConfig::default()
        })
        .seeds(seeds.clone())
        .reconnect_attempts(2)
        .discovery_timeout(Duration::from_secs(30))
        .dht_overlay_key(dht_overlay_key)
        // Bootstrap alone registers one session per config seed plus one per
        // discovered member, which already exceeds the default budget of 30 and
        // leaves the growth loop nothing to do.  A larger budget lets growth add
        // members reached transitively, which is what raises the number of peers
        // that can verify this node and push broadcasts to it.
        .overlay_max_peers(overlay_max_peers)
        .member_endpoints(member_endpoints.clone())
        // Enable quality eviction with aggressive churn so managed slots rotate
        // during the 300 s wait, letting growth continuously add fresh full-node
        // contacts (each eviction -> growth round -> new admission ticket).
        .peer_quality(PeerQualityConfig {
            churn_interval: Duration::from_secs(3),
            sweep_interval: Duration::from_secs(1),
            ..PeerQualityConfig::default()
        })
        .quality_eviction(true)
        .native_udp("0.0.0.0:0".parse().unwrap(), local_key, None)
        .start()
        .await
        .unwrap_or_else(|error| panic!("live mempool phase=start failed: {error}"));
    let connected_peers = manager.peer_count().await;
    assert!(
        connected_peers > 0,
        "live mempool phase=bootstrap readiness failed: registered_peers=0"
    );
    eprintln!(
        "live mempool phase=bootstrap elapsed={:?} registered_peers={connected_peers}",
        bootstrap_started.elapsed()
    );
    eprintln!(
        "live mempool phase=discovery {:?}",
        scanner.discovery_stats()
    );
    eprintln!("live mempool phase=connected registered_peers={connected_peers} overlay={overlay}");

    // Pre-wait growth burst: run 4 aggressive growth rounds back-to-back
    // to seed neighbour slots on full nodes before the passive wait window.
    // Uses the same UDP overlay lookup that the builder's growth task uses,
    // and resolves the whole published overlay-node member set per round
    // (the lookup bounds the round by its deadline, not by a record cap).
    for burst in 1..=4 {
        eprintln!("live mempool phase=growth-burst round={burst}");
        let burst_lookup = udp_overlay_lookup_with_endpoints(
            "0.0.0.0:0".parse().unwrap(),
            local_key,
            overlay,
            overlay_bytes,
            256,
            Duration::from_secs(30),
            member_endpoints.clone(),
        );
        let _ = tokio::time::timeout(Duration::from_secs(30), burst_lookup(seeds.clone())).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    let mut events = Box::pin(stream);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Peers outside the configured seed list that completed an
    // authenticated overlay session with this node: bootstrap
    // connects the discovery results, and periodic growth keeps
    // adding members that answer `overlay.getRandomPeers`.
    let seed_ids: std::collections::HashSet<PeerId> = seeds.iter().map(|seed| seed.peer).collect();
    let mut non_seed_connected: Vec<PeerId> = Vec::new();
    let delivery_started = tokio::time::Instant::now();
    let event = tokio::time::timeout(Duration::from_secs(timeout), async {
        loop {
            tokio::select! {
                event = events.next() => {
                    if let Some(event) = event {
                        if let MempoolEvent::PeerStatus(PeerStatus::Connected { peer }) = &event
                            && !seed_ids.contains(peer)
                            && !non_seed_connected.contains(peer)
                        {
                            eprintln!("live mempool phase=non-seed peer connected: peer={peer:?}");
                            non_seed_connected.push(*peer);
                        }
                        if let event @ MempoolEvent::ExternalMessage { .. } = event {
                            break event;
                        }
                    }
                }
                _ = progress.tick() => {
                    eprintln!(
                        "live mempool phase=waiting metrics={:?} protocol={:?}",
                        scanner.metrics(),
                        scanner.protocol_stats()
                    );
                }
            }
        }
    })
    .await;
    let event = match event {
        Ok(event) => {
            // The our-side delivery measurement: bootstrap finished, so
            // this bounds how long a broadcast from the live overlay
            // takes to reach this node's accept path.
            eprintln!(
                "live mempool phase=delivery time_to_message={:?} metrics={:?}",
                delivery_started.elapsed(),
                scanner.metrics(),
            );
            event
        }
        Err(_) => {
            let peers = manager.peer_count().await;
            let metrics = scanner.metrics();
            let protocol = scanner.protocol_stats();
            let custom_arrived = protocol.custom_messages;
            panic!(
                "live mempool phase=delivery timed out after {timeout}s: registered_peers={peers} metrics={metrics:?} protocol={protocol:?}; inbound custom overlay datagrams arrived={custom_arrived} in this window; delivery requires a verified-neighbour holder to sample this node into its broadcast fan-out (upstream get_neighbours(propagate_broadcast_to() = 5)), so a red window is an upstream sampling outcome, not a client defect - mechanism and recorded arms in docs/reference/network/overlay.md"
            );
        }
    };
    let lazy = event
        .lazy_message()
        .expect("external event must expose lazy message");
    let message = lazy
        .decode()
        .expect("live payload must decode as a TL-B message");
    assert!(matches!(
        message.info,
        tonutils_tlb::CommonMsgInfo::ExternalIn { .. }
    ));
    // Non-seed connectivity gate: the scanner must have completed
    // an authenticated overlay session with at least one peer
    // outside the configured seed list.  The seeds are
    // `dht-server` deployments that do not run overlay sessions,
    // so every Connected peer here came from discovery or growth.
    assert!(
        !non_seed_connected.is_empty(),
        "live mempool phase=non-seed: no peer outside the seed list connected (seed_count={} registered_peers={})",
        seeds.len(),
        manager.peer_count().await
    );
    eprintln!(
        "live mempool phase=non-seed connected={} seed_count={}",
        non_seed_connected.len(),
        seeds.len()
    );
    // Round-trip proof against non-seed members: open direct
    // ADNL/UDP sessions to registry members the discovery path
    // resolved - never configured seeds - and ask them for
    // random peers, the same query the growth path uses. A
    // member's recorded endpoint can go stale between discovery
    // and this check (NAT rebinding) and any single member can
    // be transiently unresponsive, so a handful of members are
    // probed and at least one must answer.
    let non_seed_members: Vec<(PeerId, SocketAddr)> = known_member_seeds(&member_endpoints)
        .into_iter()
        .filter(|(peer, _)| !seed_ids.contains(peer))
        .collect();
    assert!(
        !non_seed_members.is_empty(),
        "live mempool phase=non-seed: the member registry holds only configured seeds (seed_count={})",
        seeds.len()
    );
    // A member's recorded endpoint can go stale between discovery and
    // this check and any single member can be transiently unresponsive,
    // so several members are probed concurrently in one round and at
    // least one must answer.  Concurrency matters twice over: the whole
    // probe round costs one timeout window instead of one per target,
    // and more targets are affordable within it.
    let roundtrip_budget = non_seed_members.len().min(6);
    let probes = non_seed_members
        .into_iter()
        .take(roundtrip_budget)
        .map(|(member_peer, member_addr)| async move {
            let member_key = match PublicKey::from_bytes(member_peer.as_bytes()) {
                Some(member_key) => member_key,
                None => {
                    eprintln!(
                        "live mempool phase=non-seed udp roundtrip target={member_addr} result=invalid-key"
                    );
                    return 0usize;
                }
            };
            let roundtrip_key = KeyPair::generate(&mut rand::rngs::OsRng);
            let session = match tokio::time::timeout(
                Duration::from_secs(30),
                AdnlUdpSession::connect(
                    "0.0.0.0:0".parse().expect("wildcard address must parse"),
                    member_addr,
                    roundtrip_key,
                    member_key,
                ),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Ok(Err(error)) => {
                    eprintln!(
                        "live mempool phase=non-seed udp roundtrip target={member_addr} result=connect_error error={error}"
                    );
                    return 0;
                }
                Err(_) => {
                    eprintln!(
                        "live mempool phase=non-seed udp roundtrip target={member_addr} result=connect_timeout"
                    );
                    return 0;
                }
            };
            let nodes = match tokio::time::timeout(
                Duration::from_secs(15),
                session.overlay_get_random_peers(Int256(overlay.as_bytes()), Duration::from_secs(10)),
            )
            .await
            {
                Ok(Ok(nodes)) => nodes,
                Ok(Err(error)) => {
                    eprintln!(
                        "live mempool phase=non-seed udp roundtrip target={member_addr} result=query_error error={error}"
                    );
                    return 0;
                }
                Err(_) => {
                    eprintln!(
                        "live mempool phase=non-seed udp roundtrip target={member_addr} result=query_timeout"
                    );
                    return 0;
                }
            };
            eprintln!(
                "live mempool phase=non-seed udp roundtrip target={member_addr} result=ok random_peers={}",
                nodes.nodes.len()
            );
            1
        });
    let roundtrip_ok: usize = join_all(probes).await.into_iter().sum();
    assert!(
        roundtrip_ok > 0,
        "live mempool phase=non-seed udp roundtrip: no non-seed member answered overlay.getRandomPeers ({roundtrip_budget} probed, seed_count={})",
        seeds.len()
    );
    manager.shutdown_wait().await;
}

/// Runs the native ADNL/UDP DHT overlay discovery (`udp_overlay_lookup`)
/// against live seeds and requires at least one usable overlay peer.
///
/// Soft-skips when the network is unavailable and
/// `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` is set.
#[tokio::test]
#[ignore = "requires live mainnet DHT access"]
async fn mainnet_overlay_dht_discovery_returns_peers() {
    let _ = pretty_env_logger::try_init();
    let allow_unavailable = std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").is_ok();
    let overlay = OverlayId::from_bytes(discovery_overlay_id());
    let lookup = udp_overlay_lookup(
        "0.0.0.0:0".parse().expect("wildcard address must parse"),
        KeyPair::generate(&mut rand::rngs::OsRng),
        overlay,
        overlay.as_bytes(),
        16,
        Duration::from_secs(30),
    );
    let peers: Vec<SeedPeer> =
        tokio::time::timeout(Duration::from_secs(45), lookup(configured_live_seeds()))
            .await
            .unwrap_or_default();
    assert_discovery_peers("udp", peers, allow_unavailable);
}

#[test]
fn configured_seed_parser_accepts_ipv4_and_ipv6() {
    for value in ["127.0.0.1:30303", "[::1]:30303"] {
        assert!(SocketAddr::from_str(value).is_ok());
    }
}

#[test]
fn configured_seed_parser_ignores_empty_separators() {
    let key = "11".repeat(32);
    let seeds = parse_live_seeds(&format!(";{key}@127.0.0.1:30303;;"));
    assert_eq!(seeds.len(), 1);
    assert!(seeds[0].is_valid());
}
