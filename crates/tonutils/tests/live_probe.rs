#![cfg(all(feature = "mempool", feature = "overlay-config"))]

//! Mandatory live probes for DHT, ADNL/UDP, and QUIC request-response paths.

mod live_common;

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use futures::future::join_all;
use live_common::{
    CONNECT_TIMEOUT, DISCOVERY_OUTER_TIMEOUT, DISCOVERY_TIMEOUT, QUERY_TIMEOUT,
    configured_overlay_id, configured_seeds, elapsed_millis, load_global_config, parse_socket_addr,
    valid_overlay_nodes,
};
use tonutils::{
    adnl::{AdnlKeyPair, AdnlPublicKey, AdnlUdpSession, QuicSession, now_i32, tl::DhtNodesBoxed},
    network_config,
    overlay::{
        OverlayId, OverlayMemberEndpoints, PeerId, QuicEndpointSource, SeedPeer,
        known_member_seeds, udp_overlay_lookup_with_endpoints,
    },
    tl::TlInt256 as Int256,
};

const MAX_DHT_SEEDS: usize = 16;
const MAX_DIRECT_TARGETS: usize = 12;
const QUIC_PORT_OFFSET: u16 = 1000;
const MAX_QUIC_WAVES: usize = 5;
const QUIC_WAVE_PAUSE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
struct QuicTarget {
    peer: PeerId,
    address: SocketAddr,
    source: QuicEndpointSource,
}

#[derive(Debug)]
struct ProbeSuccess {
    elapsed_ms: u128,
    nodes: usize,
    valid: usize,
}

#[tokio::test]
#[ignore = "requires live mainnet network access"]
async fn mainnet_dht_find_node_returns_valid_nodes() -> Result<()> {
    probe_dht("mainnet", "TON_GLOBAL_CONFIG_JSON").await
}

#[tokio::test]
#[ignore = "requires live testnet network access"]
async fn testnet_dht_find_node_returns_valid_nodes() -> Result<()> {
    probe_dht("testnet", "TON_TESTNET_GLOBAL_CONFIG_JSON").await
}

#[tokio::test]
#[ignore = "requires live mainnet network access"]
async fn mainnet_udp_overlay_get_random_peers_returns_valid_nodes() -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let overlay = configured_overlay_id()?;
    let seeds = configured_seeds("TON_GLOBAL_CONFIG_JSON")?;
    let endpoints = OverlayMemberEndpoints::default();
    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let lookup = udp_overlay_lookup_with_endpoints(
        "0.0.0.0:0".parse().expect("wildcard address must parse"),
        keypair,
        overlay,
        overlay.as_bytes(),
        64,
        DISCOVERY_TIMEOUT,
        Arc::clone(&endpoints),
    );
    let started = Instant::now();
    let discovered = tokio::time::timeout(DISCOVERY_OUTER_TIMEOUT, lookup(seeds.clone()))
        .await
        .context("UDP overlay discovery timed out")?;
    eprintln!(
        "live probe kind=udp phase=discovery target=config-seeds seeds={} members={} peers={} elapsed_ms={}",
        seeds.len(),
        endpoints.lock().map(|map| map.len()).unwrap_or(0),
        discovered.len(),
        elapsed_millis(started),
    );
    if discovered.is_empty() {
        bail!("UDP overlay discovery returned no peers");
    }

    let local_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let probes = discovered
        .iter()
        .take(MAX_DIRECT_TARGETS)
        .cloned()
        .map(|peer| direct_udp_query(peer, &local_key, overlay));
    let results = join_all(probes).await;
    for (target, result) in &results {
        match result {
            Ok(success) => eprintln!(
                "live probe kind=udp request=overlay.getRandomPeers target={target} result=ok elapsed_ms={} nodes={} valid={}",
                success.elapsed_ms, success.nodes, success.valid,
            ),
            Err(error) => eprintln!(
                "live probe kind=udp request=overlay.getRandomPeers target={target} result=error error={error}"
            ),
        }
    }
    if let Some((target, Ok(success))) = results.into_iter().find(|(_, result)| result.is_ok()) {
        eprintln!(
            "live probe kind=udp request=overlay.getRandomPeers result=success target={target} elapsed_ms={} nodes={} valid={}",
            success.elapsed_ms, success.nodes, success.valid,
        );
        return Ok(());
    }
    bail!("no discovered UDP member returned valid nonempty overlay.getRandomPeers");
}

#[tokio::test]
#[ignore = "requires live mainnet network access"]
async fn mainnet_quic_overlay_get_random_peers_returns_valid_nodes() -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let overlay = configured_overlay_id()?;
    let seeds = configured_seeds("TON_GLOBAL_CONFIG_JSON")?;
    let endpoints = OverlayMemberEndpoints::default();
    let discovery_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let lookup = udp_overlay_lookup_with_endpoints(
        "0.0.0.0:0".parse().expect("wildcard address must parse"),
        discovery_key,
        overlay,
        overlay.as_bytes(),
        64,
        DISCOVERY_TIMEOUT,
        Arc::clone(&endpoints),
    );
    let started = Instant::now();
    let discovered = tokio::time::timeout(DISCOVERY_OUTER_TIMEOUT, lookup(seeds.clone()))
        .await
        .context("UDP discovery before QUIC probe timed out")?;
    let registered = endpoints.lock().map(|map| map.len()).unwrap_or(0);
    eprintln!(
        "live probe kind=quic phase=udp-discovery target=config-seeds seeds={} members={registered} peers={} elapsed_ms={}",
        seeds.len(),
        discovered.len(),
        elapsed_millis(started),
    );
    if registered == 0 {
        bail!("UDP discovery registered no overlay members for QUIC");
    }

    let mut pending = quic_targets(&endpoints);
    if pending.is_empty() {
        bail!("no advertised or udp+1000 QUIC targets were derived");
    }
    let mut timeouts = 0usize;
    let mut errors = 0usize;
    for wave in 1..=MAX_QUIC_WAVES {
        if wave > 1 {
            tokio::time::sleep(QUIC_WAVE_PAUSE).await;
        }
        eprintln!(
            "live probe kind=quic phase=connect wave={wave} targets={}",
            pending.len()
        );
        let attempts = pending
            .iter()
            .copied()
            .map(|target| async move { (target, direct_quic_query(target, overlay).await) });
        let results = join_all(attempts).await;
        let mut failed = Vec::new();
        for (target, result) in results {
            match result {
                Ok(success) => {
                    eprintln!(
                        "live probe kind=quic phase=connect request=overlay.getRandomPeers target={} peer={} source={} wave={wave} result=ok elapsed_ms={} nodes={} valid={}",
                        target.address,
                        hex::encode(target.peer.as_bytes()),
                        target.source.label(),
                        success.elapsed_ms,
                        success.nodes,
                        success.valid,
                    );
                    return Ok(());
                }
                Err(error) if error == "timeout" => {
                    timeouts += 1;
                    failed.push(target);
                    eprintln!(
                        "live probe kind=quic phase=connect target={} peer={} source={} wave={wave} result=timeout",
                        target.address,
                        hex::encode(target.peer.as_bytes()),
                        target.source.label(),
                    );
                }
                Err(error) => {
                    errors += 1;
                    failed.push(target);
                    eprintln!(
                        "live probe kind=quic phase=connect target={} peer={} source={} wave={wave} result=error error={error}",
                        target.address,
                        hex::encode(target.peer.as_bytes()),
                        target.source.label(),
                    );
                }
            }
        }
        pending = failed;
        if pending.is_empty() {
            break;
        }
    }
    bail!(
        "no discovered QUIC target returned valid nonempty overlay.getRandomPeers \
         (waves={MAX_QUIC_WAVES}, timeouts={timeouts}, errors={errors})"
    )
}

async fn probe_dht(network: &str, variable: &str) -> Result<()> {
    let _ = pretty_env_logger::try_init();
    let json = load_global_config(variable)?;
    let candidates = network_config::extract_dht_addresses(&json)
        .with_context(|| format!("{variable} has no DHT static nodes"))?;
    let local_key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let mut errors = Vec::new();
    for candidate in candidates.into_iter().take(MAX_DHT_SEEDS) {
        let Some(public_key) = candidate.public_key else {
            errors.push(format!("{}: missing public key", candidate.address));
            continue;
        };
        let Some(remote_key) = AdnlPublicKey::from_bytes(public_key) else {
            errors.push(format!("{}: invalid public key", candidate.address));
            continue;
        };
        let started = Instant::now();
        let session = tokio::time::timeout(
            CONNECT_TIMEOUT,
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().expect("wildcard address must parse"),
                candidate.address,
                local_key,
                remote_key,
            ),
        )
        .await;
        let session = match session {
            Err(_) => {
                errors.push(format!("{}: connect timeout", candidate.address));
                continue;
            }
            Ok(Err(error)) => {
                errors.push(format!("{}: connect error {error}", candidate.address));
                continue;
            }
            Ok(Ok(session)) => session,
        };
        let answer = tokio::time::timeout(
            QUERY_TIMEOUT,
            session.dht_find_node(Int256::random(), 8, CONNECT_TIMEOUT),
        )
        .await;
        match answer {
            Err(_) => errors.push(format!("{}: findNode timeout", candidate.address)),
            Ok(Err(error)) => {
                errors.push(format!("{}: findNode error {error}", candidate.address));
            }
            Ok(Ok(nodes)) => {
                let valid = valid_dht_nodes(&nodes);
                eprintln!(
                    "live probe kind=dht network={network} request=dht.findNode target={} peer={} elapsed_ms={} nodes={} valid={}",
                    candidate.address,
                    hex::encode(public_key),
                    elapsed_millis(started),
                    nodes.nodes.len(),
                    valid,
                );
                if !nodes.nodes.is_empty() && valid == nodes.nodes.len() {
                    return Ok(());
                }
                errors.push(format!(
                    "{}: answer nodes={} valid={valid}",
                    candidate.address,
                    nodes.nodes.len()
                ));
            }
        }
    }
    bail!(
        "no {network} DHT seed returned valid nonempty dht.findNode: {}",
        errors.join("; ")
    )
}

fn valid_dht_nodes(nodes: &DhtNodesBoxed) -> usize {
    let now = now_i32();
    nodes.nodes.iter().filter(|node| node.is_valid(now)).count()
}

async fn direct_udp_query(
    peer: SeedPeer,
    local_key: &AdnlKeyPair,
    overlay: OverlayId,
) -> (String, Result<ProbeSuccess>) {
    let target = peer.address.clone();
    let result = async {
        let remote_key = AdnlPublicKey::from_bytes(peer.peer.as_bytes())
            .context("discovered member has an invalid public key")?;
        let address = parse_socket_addr(&peer.address)?;
        let started = Instant::now();
        let session = tokio::time::timeout(
            CONNECT_TIMEOUT,
            AdnlUdpSession::connect(
                "0.0.0.0:0".parse().expect("wildcard address must parse"),
                address,
                *local_key,
                remote_key,
            ),
        )
        .await
        .context("UDP connect timeout")?
        .map_err(anyhow::Error::msg)?;
        let nodes = tokio::time::timeout(
            QUERY_TIMEOUT,
            session.overlay_get_random_peers(Int256(overlay.as_bytes()), QUERY_TIMEOUT),
        )
        .await
        .context("UDP overlay.getRandomPeers timeout")?
        .map_err(anyhow::Error::msg)?;
        let valid = valid_overlay_nodes(&nodes.nodes, overlay);
        if nodes.nodes.is_empty() || valid != nodes.nodes.len() {
            bail!(
                "UDP overlay.getRandomPeers returned nodes={} valid={valid}",
                nodes.nodes.len()
            );
        }
        Ok(ProbeSuccess {
            elapsed_ms: elapsed_millis(started),
            nodes: nodes.nodes.len(),
            valid,
        })
    }
    .await;
    (target, result)
}

async fn direct_quic_query(target: QuicTarget, overlay: OverlayId) -> Result<ProbeSuccess, String> {
    let remote_key = AdnlPublicKey::from_bytes(target.peer.as_bytes())
        .ok_or_else(|| "QUIC target has an invalid public key".to_owned())?;
    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let started = Instant::now();
    let connection = tokio::time::timeout(
        QUERY_TIMEOUT,
        QuicSession::connect(
            "0.0.0.0:0".parse().expect("wildcard address must parse"),
            target.address,
            keypair,
            remote_key,
        ),
    )
    .await
    .map_err(|_| "timeout".to_owned())?
    .map_err(|error| format!("QUIC connect failed: {error}"))?;
    let nodes = tokio::time::timeout(
        QUERY_TIMEOUT,
        connection.overlay_get_random_peers(Int256(overlay.as_bytes()), QUERY_TIMEOUT),
    )
    .await
    .map_err(|_| "timeout".to_owned())?
    .map_err(|error| format!("QUIC overlay.getRandomPeers failed: {error}"))?;
    let valid = valid_overlay_nodes(&nodes.nodes, overlay);
    if nodes.nodes.is_empty() || valid != nodes.nodes.len() {
        return Err(format!(
            "QUIC overlay.getRandomPeers returned nodes={} valid={valid}",
            nodes.nodes.len()
        ));
    }
    Ok(ProbeSuccess {
        elapsed_ms: elapsed_millis(started),
        nodes: nodes.nodes.len(),
        valid,
    })
}

fn quic_targets(endpoints: &OverlayMemberEndpoints) -> Vec<QuicTarget> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    for (peer, udp_or_advertised) in known_member_seeds(endpoints) {
        let entry = endpoints
            .lock()
            .ok()
            .and_then(|map| map.get(&peer).copied());
        if let Some(advertised) = entry.and_then(|entry| entry.quic) {
            push_quic_target(
                &mut targets,
                &mut seen,
                QuicTarget {
                    peer,
                    address: advertised,
                    source: QuicEndpointSource::Advertised,
                },
            );
        }
        let udp = entry
            .and_then(|entry| entry.udp)
            .unwrap_or(udp_or_advertised);
        let offset = SocketAddr::new(udp.ip(), udp.port().wrapping_add(QUIC_PORT_OFFSET));
        push_quic_target(
            &mut targets,
            &mut seen,
            QuicTarget {
                peer,
                address: offset,
                source: QuicEndpointSource::UdpPortOffset,
            },
        );
    }
    targets
}

fn push_quic_target(
    targets: &mut Vec<QuicTarget>,
    seen: &mut HashSet<(PeerId, SocketAddr)>,
    target: QuicTarget,
) {
    if seen.insert((target.peer, target.address)) {
        targets.push(target);
    }
}
