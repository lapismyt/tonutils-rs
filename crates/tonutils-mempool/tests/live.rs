use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use futures::future::join_all;
use tonutils_adnl::AdnlUdpSession;
use tonutils_adnl::{KeyPair, PublicKey};
use tonutils_mempool::MempoolScannerBuilder;
use tonutils_mempool::{
    MempoolConfig, MempoolEvent, OverlayMemberEndpoints, known_member_seeds, member_quic_target,
    quic_overlay_lookup_with_endpoints, registered_member_count, udp_overlay_lookup,
    udp_overlay_lookup_with_endpoints,
};
use tonutils_network_config::extract_dht_addresses;
use tonutils_overlay::{OverlayId, PeerId, SeedPeer};
use tonutils_tl::Int256;
use tonutils_tl::tl::adnl::Message as AdnlMessage;
use tonutils_tl::tl::network::{DhtMessage, DhtNodesBoxed, PacketContents};

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

fn configured_bytes(variable: &str) -> Option<[u8; 32]> {
    let value = match std::env::var(variable) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return None,
        Err(error) => panic!("failed to read {variable}: {error}"),
    };
    if value.trim().is_empty() {
        return None;
    }
    let bytes =
        hex::decode(value).unwrap_or_else(|error| panic!("{variable} must be hex: {error}"));
    bytes
        .try_into()
        .ok()
        .or_else(|| panic!("{variable} must contain 32 bytes"))
}

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
    let (scanner, manager, stream) = MempoolScannerBuilder::new()
        .download_config(false)
        .overlay_id(overlay)
        .config(MempoolConfig::default())
        .seeds(seeds)
        .reconnect_attempts(2)
        .discovery_timeout(Duration::from_secs(30))
        .dht_overlay_key(dht_overlay_key)
        // Bootstrap alone registers one session per config seed plus one per
        // discovered member, which already exceeds the default budget of 30 and
        // leaves the growth loop nothing to do.  A larger budget lets growth add
        // members reached transitively, which is what raises the number of peers
        // that can verify this node and push broadcasts to it.
        .overlay_max_peers(overlay_max_peers)
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
    let event = tokio::time::timeout(Duration::from_secs(timeout), async {
        loop {
            tokio::select! {
                event = events.next() => {
                    if let Some(event @ MempoolEvent::ExternalMessage { .. }) = event {
                        break event;
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
    manager.shutdown_wait().await;
}

/// Resolves the seed set shared by the mempool live tests: either the
/// `;`-separated `TON_MEMPOOL_LIVE_SEEDS` list, or a single
/// `TON_MEMPOOL_LIVE_SEED` address paired with `TON_MEMPOOL_LIVE_PEER_KEY`.
fn configured_live_seeds() -> Vec<SeedPeer> {
    match std::env::var("TON_MEMPOOL_LIVE_SEEDS") {
        Ok(value) if !value.trim().is_empty() => parse_live_seeds(&value),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            let seed_address: SocketAddr = match std::env::var("TON_MEMPOOL_LIVE_SEED") {
                Ok(value) => value
                    .parse()
                    .expect("TON_MEMPOOL_LIVE_SEED must be IP:port"),
                Err(error) => panic!("TON_MEMPOOL_LIVE_SEED must be set to IP:port: {error}"),
            };
            let peer_key = configured_bytes("TON_MEMPOOL_LIVE_PEER_KEY")
                .expect("TON_MEMPOOL_LIVE_PEER_KEY must be set to 32-byte hex");
            vec![SeedPeer {
                peer: PeerId::from_bytes(peer_key),
                address: seed_address.to_string(),
            }]
        }
        Err(error) => panic!("failed to read TON_MEMPOOL_LIVE_SEEDS: {error}"),
    }
}

/// Overlay id used by the discovery live tests, defaulting to the mainnet
/// mempool overlay that CI also uses for the delivery test.
fn discovery_overlay_id() -> [u8; 32] {
    configured_bytes("TON_MEMPOOL_LIVE_OVERLAY_ID").unwrap_or_else(|| {
        let hex = "12b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649";
        hex::decode(hex)
            .expect("static overlay id must be hex")
            .try_into()
            .expect("static overlay id must contain 32 bytes")
    })
}

/// Shared assertion for the discovery live tests: a discovery run either
/// yields usable peers or, with `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` set,
/// logs and passes instead of failing on transport unavailability.
fn assert_discovery_peers(kind: &str, peers: Vec<SeedPeer>, allow_unavailable: bool) {
    eprintln!(
        "live overlay discovery kind={kind} phase=lookup peers={}",
        peers.len()
    );
    if peers.is_empty() {
        if allow_unavailable {
            log::debug!("skipping unavailable {kind} overlay discovery: no peers");
            return;
        }
        panic!("{kind} overlay discovery returned no peers");
    }
    assert!(
        peers.iter().all(SeedPeer::is_valid),
        "{kind} overlay discovery must return usable KEY@IP:PORT peers"
    );
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
/// Soft-skips - with an explicit `members=`/`quic_peers=` log - when either
/// phase finds nothing and `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` is set.
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
    let targets: Vec<SocketAddr> = known_member_seeds(&endpoints)
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
}

fn parse_live_seeds(value: &str) -> Vec<SeedPeer> {
    value
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            let (key, address) = entry
                .split_once('@')
                .unwrap_or_else(|| panic!("live seed must use KEY@IP:PORT: {entry}"));
            let peer_key = hex::decode(key).expect("live seed key must be hex");
            let peer_key: [u8; 32] = peer_key
                .try_into()
                .expect("live seed key must contain 32 bytes");
            address
                .parse::<SocketAddr>()
                .unwrap_or_else(|_| panic!("live seed address must be IP:PORT: {address}"));
            SeedPeer::from_public_key(peer_key, address)
        })
        .collect()
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
