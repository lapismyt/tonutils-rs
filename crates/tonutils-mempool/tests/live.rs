use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use futures::future::join_all;
use tonutils_adnl::{AdnlUdpSession, KeyPair, PublicKey, QuicSession};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_mempool::{
    MempoolConfig, MempoolEvent, OverlayMemberEndpoints, known_member_seeds, member_quic_target,
    quic_overlay_lookup_with_endpoints, registered_member_count, udp_overlay_lookup,
    udp_overlay_lookup_with_endpoints,
};
use tonutils_network_config::extract_dht_addresses;
use tonutils_overlay::{OverlayId, PeerId, PeerStatus, SeedPeer};
use tonutils_tl::Int256;
use tonutils_tl::tl::adnl::Message as AdnlMessage;
use tonutils_tl::tl::network::{DhtMessage, DhtNodesBoxed, PacketContents};

// Shared environment parsing and soft-skip assertion for the
// live tests in this directory; re-exported so call sites read
// unchanged.
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
    // Shared with the builder so the UDP discovery, peer-growth and
    // session factory all record into the registry this test reads
    // after bootstrap: it holds every runtime-resolved member, which
    // is the non-seed peer set the connectivity checks below dial.
    let member_endpoints = OverlayMemberEndpoints::default();
    let (scanner, manager, stream) = MempoolScannerBuilder::new()
        .download_config(false)
        .overlay_id(overlay)
        .config(MempoolConfig::default())
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
        "live mempool phase=discovery {:?}",
        scanner.discovery_stats()
    );
    eprintln!("live mempool phase=connected registered_peers={connected_peers} overlay={overlay}");
    let mut events = Box::pin(stream);
    let mut progress = tokio::time::interval(Duration::from_secs(10));
    progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Peers outside the configured seed list that completed an
    // authenticated overlay session with this node: bootstrap
    // connects the discovery results, and periodic growth keeps
    // adding members that answer `overlay.getRandomPeers`.
    let seed_ids: std::collections::HashSet<PeerId> = seeds.iter().map(|seed| seed.peer).collect();
    let mut non_seed_connected: Vec<PeerId> = Vec::new();
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
        Ok(event) => event,
        Err(_) => {
            let peers = manager.peer_count().await;
            let metrics = scanner.metrics();
            let protocol = scanner.protocol_stats();
            panic!(
                "live mempool phase=delivery timed out after {timeout}s: registered_peers={peers} metrics={metrics:?} protocol={protocol:?}"
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
    let roundtrip_budget = non_seed_members.len().min(3);
    let mut roundtrip_ok = 0usize;
    for (member_peer, member_addr) in non_seed_members.into_iter().take(roundtrip_budget) {
        let member_key = match PublicKey::from_bytes(member_peer.as_bytes()) {
            Some(member_key) => member_key,
            None => {
                eprintln!(
                    "live mempool phase=non-seed udp roundtrip target={member_addr} result=invalid-key"
                );
                continue;
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
                continue;
            }
            Err(_) => {
                eprintln!(
                    "live mempool phase=non-seed udp roundtrip target={member_addr} result=connect_timeout"
                );
                continue;
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
                continue;
            }
            Err(_) => {
                eprintln!(
                    "live mempool phase=non-seed udp roundtrip target={member_addr} result=query_timeout"
                );
                continue;
            }
        };
        roundtrip_ok += 1;
        eprintln!(
            "live mempool phase=non-seed udp roundtrip target={member_addr} result=ok random_peers={}",
            nodes.nodes.len()
        );
    }
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

// ---------------------------------------------------------------------------
// Raw QUIC Version Negotiation probes (RFC 9000 ss 5.2.2, 6.1, 17.2.1).
//
// A QUIC handshake timeout alone cannot distinguish "nothing answers UDP at
// this port" from "a QUIC endpoint answers but our handshake is rejected".
// A Version Negotiation probe elicits an answer from any RFC-compliant QUIC
// listener regardless of version support, so `vn_replies` separates
// reachability from handshake failures, and the control targets show whether
// this runner's UDP egress can reach public QUIC servers at all.
// ---------------------------------------------------------------------------

/// QUIC port offset: upstream TON full nodes listen on `adnl_udp_port +
/// NODE_PORT_OFFSET` where `NODE_PORT_OFFSET = 1000` (`quic/quic-sender.h`).
/// Mirrors `tonutils_mempool`'s private `QUIC_PORT_OFFSET` so the probe
/// targets exactly what the QUIC lookup dials.
const QUIC_PORT_OFFSET: u16 = 1000;

/// Connection ids sent by [`version_negotiation_probe`] and echoed back by a
/// compliant server (RFC 9000 ss 17.2.1), so replies can be matched to the
/// probe instead of trusting any stray datagram.
const VN_DCID: [u8; 8] = *b"tonvndc1";
const VN_SCID: [u8; 8] = *b"tonvnsi1";

/// Builds one raw Version Negotiation probe datagram.
///
/// RFC 9000 ss 15 reserves versions of the `0x0a0a0a0a` pattern for "forcing
/// version negotiation to be exercised", and ss 5.2.2 lets servers drop
/// unsupported-version packets smaller than the largest minimum datagram
/// (1200 bytes for QUIC v1 and v2), so the probe is padded to exactly 1200
/// bytes before any server is obliged to answer.
fn version_negotiation_probe_packet() -> Vec<u8> {
    let mut packet = Vec::with_capacity(1200);
    // Long header form with the fixed bit set; the type bits are
    // version-specific and ignored for unsupported versions (RFC 9000 ss 17.2).
    packet.push(0xc0);
    packet.extend_from_slice(&0x0a0a0a0a_u32.to_be_bytes());
    packet.push(VN_DCID.len() as u8);
    packet.extend_from_slice(&VN_DCID);
    packet.push(VN_SCID.len() as u8);
    packet.extend_from_slice(&VN_SCID);
    packet.resize(1200, 0);
    packet
}

/// Parses a Version Negotiation reply (RFC 9000 ss 17.2.1), validating that
/// the server echoed the probe's connection ids and that the payload lists
/// whole 32-bit versions. Returns the advertised versions on success.
fn parse_version_negotiation(reply: &[u8]) -> Option<Vec<u32>> {
    if reply.len() < 7 || reply[0] & 0x80 == 0 {
        return None;
    }
    if reply[1..5] != [0, 0, 0, 0] {
        return None;
    }
    // The server must echo our SCID as the reply's DCID and our DCID as the
    // reply's SCID.
    let dcid_len = usize::from(reply[5]);
    let scid_len_at = 6 + dcid_len;
    if reply.len() <= scid_len_at || reply[6..scid_len_at] != VN_SCID[..] {
        return None;
    }
    let scid_len = usize::from(reply[scid_len_at]);
    let versions_at = scid_len_at + 1 + scid_len;
    if reply.len() < versions_at || reply[scid_len_at + 1..versions_at] != VN_DCID[..] {
        return None;
    }
    let versions = &reply[versions_at..];
    if versions.is_empty() || !versions.len().is_multiple_of(4) {
        return None;
    }
    Some(
        versions
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_be_bytes(*chunk))
            .collect(),
    )
}

/// Sends one raw Version Negotiation probe to `target` and returns the
/// advertised versions, or a short reason string when the target does not
/// answer within `timeout` with a reply that echoes this probe.
async fn version_negotiation_probe(
    target: SocketAddr,
    timeout: Duration,
) -> Result<Vec<u32>, String> {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|error| error.to_string())?;
    socket
        .send_to(&version_negotiation_probe_packet(), target)
        .await
        .map_err(|error| error.to_string())?;

    let mut buffer = [0u8; 1500];
    match tokio::time::timeout(timeout, socket.recv_from(&mut buffer)).await {
        Err(_) => Err("timeout".into()),
        Ok(Err(error)) => Err(error.to_string()),
        Ok(Ok((length, _peer))) => parse_version_negotiation(&buffer[..length])
            .ok_or_else(|| "reply was not a matching Version Negotiation packet".into()),
    }
}

/// Probes every target concurrently and returns `(target, result)` pairs.
async fn probe_all(
    targets: Vec<SocketAddr>,
    timeout: Duration,
) -> Vec<(SocketAddr, Result<Vec<u32>, String>)> {
    join_all(targets.into_iter().map(|target| async move {
        let result = version_negotiation_probe(target, timeout).await;
        match &result {
            Ok(versions) => {
                let versions: Vec<String> = versions
                    .iter()
                    .map(|version| format!("{version:08x}"))
                    .collect();
                eprintln!(
                    "live overlay discovery kind=quic vn target={target} reply=versions={}",
                    versions.join(",")
                );
            }
            Err(error) => eprintln!(
                "live overlay discovery kind=quic vn target={target} reply=none reason={error}"
            ),
        }
        (target, result)
    }))
    .await
}

/// QUIC overlay discovery against the members the UDP path resolves first.
///
/// The bootstrap seeds are `dht-server` deployments, which run no QUIC
/// listener at all (`dht-server/*.cpp` contains no QUIC code), while upstream
/// full nodes bind a `QuicServer` on `adnl_udp_port + 1000`
/// (`quic/quic-sender.h::NODE_PORT_OFFSET`).  Phase 1 therefore runs the
/// proven UDP discovery to populate a shared
/// [`OverlayMemberEndpoints`] registry; phase 1.5 sends raw Version
/// Negotiation probes to the same targets (with `1.1.1.1:443` and
/// `8.8.8.8:443` as egress controls) to separate missing listeners from
/// handshake failures; phase 2 probes those members over QUIC and reports
/// the probe counts.
///
/// Soft-skips only the discovery phases - with an explicit
/// `members=`/`quic_peers=` log - when they find nothing and
/// `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` is set.  The connect phase
/// hard-requires at least one runtime-discovered member to answer a
/// QUIC handshake, because that is the reachability claim this test
/// exists to check.
#[tokio::test]
#[ignore = "requires live mainnet QUIC access"]
async fn mainnet_overlay_quic_discovery_returns_peers() {
    let _ = pretty_env_logger::try_init();
    let allow_unavailable = std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").is_ok();
    let overlay = OverlayId::from_bytes(discovery_overlay_id());
    let local_addr: SocketAddr = "0.0.0.0:0".parse().expect("wildcard address must parse");
    let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
    let endpoints = OverlayMemberEndpoints::default();

    // Phase 1: ADNL/UDP discovery registers live overlay members and any
    // advertised `adnl.address.quic` their DHT address records carry.
    let udp_lookup = udp_overlay_lookup_with_endpoints(
        local_addr,
        keypair,
        overlay,
        overlay.as_bytes(),
        16,
        Duration::from_secs(30),
        endpoints.clone(),
    );
    let udp_peers: Vec<SeedPeer> =
        tokio::time::timeout(Duration::from_secs(45), udp_lookup(configured_live_seeds()))
            .await
            .unwrap_or_default();
    let members = registered_member_count(&endpoints);
    eprintln!(
        "live overlay discovery kind=quic phase=members registered={members} udp_peers={}",
        udp_peers.len()
    );
    if members == 0 {
        if allow_unavailable {
            log::debug!("skipping QUIC overlay discovery: no overlay members registered");
            return;
        }
        panic!("UDP discovery registered no overlay members to probe over QUIC");
    }

    // Phase 1.5: raw Version Negotiation probes separate "no listener /
    // firewalled port" from "a QUIC endpoint answers but our handshake
    // fails": any RFC-compliant QUIC server must answer the probe
    // (RFC 9000 ss 5.2.2) whatever version it supports. The controls show
    // whether this runner can reach public QUIC servers over UDP at all.
    let members_seeds = known_member_seeds(&endpoints);
    let targets: Vec<SocketAddr> = members_seeds
        .iter()
        .map(|(peer, udp)| member_quic_target(&endpoints, *peer, *udp, QUIC_PORT_OFFSET).0)
        .collect();
    let controls: Vec<SocketAddr> = ["1.1.1.1:443", "8.8.8.8:443"]
        .iter()
        .map(|address| address.parse().expect("control address must parse"))
        .collect();
    let (member_results, control_results) = tokio::join!(
        probe_all(targets, Duration::from_secs(5)),
        probe_all(controls, Duration::from_secs(5)),
    );
    let vn_replies = member_results
        .iter()
        .filter(|(_, result)| result.is_ok())
        .count();
    let controls_summary: Vec<String> = control_results
        .iter()
        .map(|(target, result)| {
            format!(
                "{target}:{}",
                if result.is_ok() { "reply" } else { "no-reply" }
            )
        })
        .collect();
    eprintln!(
        "live overlay discovery kind=quic phase=version-negotiation targets={} vn_replies={} controls={}",
        member_results.len(),
        vn_replies,
        controls_summary.join(",")
    );

    // Phase 2: probe the registered members over QUIC.
    let lookup = quic_overlay_lookup_with_endpoints(
        local_addr,
        keypair,
        overlay,
        overlay.as_bytes(),
        16,
        Duration::from_secs(30),
        endpoints.clone(),
    );
    let peers: Vec<SeedPeer> =
        tokio::time::timeout(Duration::from_secs(45), lookup(configured_live_seeds()))
            .await
            .unwrap_or_default();
    eprintln!(
        "live overlay discovery kind=quic phase=probe members_before={members} quic_peers={} members_after={}",
        peers.len(),
        registered_member_count(&endpoints)
    );
    assert_discovery_peers("quic", peers, allow_unavailable);

    // Phase 2b: direct QUIC connects to the registered members. The
    // lookup above dials members internally but reports only the peers
    // its queries discovered, not which member completed the handshake;
    // dialing each member directly proves that at least one
    // runtime-discovered member - never a configured seed - answers a
    // QUIC connect, and the connected sessions speak the overlay query
    // framing.
    let connect_results = join_all(members_seeds.iter().map(|(peer, udp)| {
        let target = member_quic_target(&endpoints, *peer, *udp, QUIC_PORT_OFFSET).0;
        let remote = PublicKey::from_bytes(peer.as_bytes());
        async move {
            let Some(remote) = remote else {
                return (target, Err("invalid member public key".to_owned()));
            };
            let attempt = tokio::time::timeout(
                Duration::from_secs(5),
                QuicSession::connect(local_addr, target, keypair, remote),
            )
            .await;
            match attempt {
                Err(_) => (target, Err("timeout".to_owned())),
                Ok(Err(error)) => (target, Err(error.to_string())),
                Ok(Ok(session)) => (target, Ok(session)),
            }
        }
    }))
    .await;
    let mut connected = 0usize;
    let mut timeouts = 0usize;
    let mut errors = 0usize;
    for (target, result) in &connect_results {
        match result {
            Ok(session) => {
                connected += 1;
                let query = session
                    .overlay_get_random_peers(Int256(overlay.as_bytes()), Duration::from_secs(15))
                    .await;
                match query {
                    Ok(nodes) => eprintln!(
                        "live overlay discovery kind=quic phase=connect target={target} result=connected random_peers={}",
                        nodes.nodes.len()
                    ),
                    Err(error) => eprintln!(
                        "live overlay discovery kind=quic phase=connect target={target} result=connected query_error={error}"
                    ),
                }
            }
            Err(error) if error == "timeout" => {
                timeouts += 1;
                eprintln!(
                    "live overlay discovery kind=quic phase=connect target={target} result=timeout"
                );
            }
            Err(error) => {
                errors += 1;
                eprintln!(
                    "live overlay discovery kind=quic phase=connect target={target} result=error error={error}"
                );
            }
        }
    }
    eprintln!(
        "live overlay discovery kind=quic phase=connect targets={} connected={connected} timeouts={timeouts} errors={errors}",
        connect_results.len()
    );
    assert!(
        connected > 0,
        "live overlay discovery kind=quic phase=connect: no runtime-discovered member answered a QUIC connect ({} targets, {timeouts} timeouts, {errors} errors)",
        connect_results.len()
    );
}

#[test]
fn configured_seed_parser_accepts_ipv4_and_ipv6() {
    for value in ["127.0.0.1:30303", "[::1]:30303"] {
        assert!(SocketAddr::from_str(value).is_ok());
    }
}

#[tokio::test]
async fn version_negotiation_probe_accepts_a_synthetic_reply() {
    let responder = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = responder.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut buffer = [0u8; 2048];
        let (length, peer) = responder.recv_from(&mut buffer).await.unwrap();
        let probe = &buffer[..length];
        assert_eq!(probe.len(), 1200, "probe must satisfy RFC 9000 ss 5.2.2");
        assert_eq!(&probe[1..5], &0x0a0a0a0a_u32.to_be_bytes());

        // Reply with the id echo mandated by RFC 9000 ss 17.2.1: the
        // reply's DCID carries the probe's SCID and vice versa.
        let dcid_len = probe[5] as usize;
        let scid_len_at = 6 + dcid_len;
        let scid_len = probe[scid_len_at] as usize;
        let mut reply = vec![0xc0u8];
        reply.extend_from_slice(&[0, 0, 0, 0]);
        reply.push(scid_len as u8);
        reply.extend_from_slice(&probe[scid_len_at + 1..scid_len_at + 1 + scid_len]);
        reply.push(dcid_len as u8);
        reply.extend_from_slice(&probe[6..scid_len_at]);
        reply.extend_from_slice(&0x00000001_u32.to_be_bytes());
        reply.extend_from_slice(&0x6b3343cf_u32.to_be_bytes());
        responder.send_to(&reply, peer).await.unwrap();
    });

    let versions = version_negotiation_probe(target, Duration::from_secs(5))
        .await
        .expect("the synthetic Version Negotiation reply must parse");
    assert_eq!(versions, vec![0x00000001, 0x6b3343cf]);
    server.await.unwrap();
}

#[tokio::test]
async fn version_negotiation_probe_times_out_without_a_reply() {
    let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = silent.local_addr().unwrap();
    let error = version_negotiation_probe(target, Duration::from_millis(200))
        .await
        .expect_err("a silent target must not answer the probe");
    assert_eq!(error, "timeout");
}

#[test]
fn version_negotiation_parser_rejects_foreign_replies() {
    // Too short for a Version Negotiation packet.
    assert_eq!(parse_version_negotiation(&[0xc0, 0, 0, 0, 0]), None);
    // Short header form (a normal QUIC packet, not version negotiation).
    assert_eq!(parse_version_negotiation(&[0x40, 0, 0, 0, 0, 0]), None);

    // A long header packet with a non-zero version is not version
    // negotiation.
    let mut wrong_version = vec![0xc0, 0, 0, 0, 1, 8];
    wrong_version.extend_from_slice(&VN_SCID);
    wrong_version.push(8);
    wrong_version.extend_from_slice(&VN_DCID);
    wrong_version.extend_from_slice(&0x00000001_u32.to_be_bytes());
    assert_eq!(parse_version_negotiation(&wrong_version), None);

    // Connection ids that do not echo this probe's ids.
    let mut wrong_ids = vec![0xc0, 0, 0, 0, 0, 8];
    wrong_ids.extend_from_slice(&[0xab; 8]);
    wrong_ids.push(8);
    wrong_ids.extend_from_slice(&[0xcd; 8]);
    wrong_ids.extend_from_slice(&0x00000001_u32.to_be_bytes());
    assert_eq!(parse_version_negotiation(&wrong_ids), None);

    // A version list truncated mid-entry.
    let mut truncated = vec![0xc0, 0, 0, 0, 0, 8];
    truncated.extend_from_slice(&VN_SCID);
    truncated.push(8);
    truncated.extend_from_slice(&VN_DCID);
    truncated.extend_from_slice(&[0x00, 0x00]);
    assert_eq!(parse_version_negotiation(&truncated), None);
}

#[test]
fn configured_seed_parser_ignores_empty_separators() {
    let key = "11".repeat(32);
    let seeds = parse_live_seeds(&format!(";{key}@127.0.0.1:30303;;"));
    assert_eq!(seeds.len(), 1);
    assert!(seeds[0].is_valid());
}
