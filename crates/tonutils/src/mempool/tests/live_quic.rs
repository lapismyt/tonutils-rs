//! Live mainnet QUIC discovery: the Version Negotiation probe helpers and
//! `mainnet_overlay_quic_discovery_returns_peers`, moved out of `live.rs` so
//! the CI step can run this binary alone (`--test live_quic`) and so the
//! non-ignored Version Negotiation unit tests below build without the rest
//! of the live suite.
//!
//! The test dials runtime-discovered overlay members, never the configured
//! `dht-server` seeds, and hard-asserts that at least one of them answers a
//! QUIC handshake. Discovery runs in two UDP rounds against one shared
//! [`OverlayMemberEndpoints`] registry, and the direct connects run in up to
//! three waves with a per-target 10 s budget; every per-target line carries
//! the `source=advertised` / `source=udp+1000` label of the endpoint that
//! was dialed, and the summary counts connected sessions per source plus the
//! number of waves used. See `docs/reference/network/quic.md` for the
//! reachability verdict behind this design.

use std::{net::SocketAddr, time::Duration};

use crate::{
    adnl::{AdnlKeyPair, AdnlPublicKey, QuicSession},
    mempool::{
        OverlayMemberEndpoints, QuicEndpointSource, known_member_seeds,
        quic_overlay_lookup_with_endpoints, record_member, record_member_quic,
        registered_member_count,
        tests::common::{assert_discovery_peers, configured_live_seeds, discovery_overlay_id},
        udp_overlay_lookup_with_endpoints,
    },
    overlay::{OverlayId, PeerId, SeedPeer},
    tl::Int256,
};

use futures::future::join_all;

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

/// Builds dial targets for ALL endpoints of every registered member:
/// - If the member has an advertised `adnl.address.quic`, include it with
///   `source=advertised`.
/// - If the member has a UDP endpoint (or was registered only via its
///   advertised QUIC, in which case `known_member_seeds` surfaced that
///   advertised address as its UDP stand-in), also include the derived
///   `udp + QUIC_PORT_OFFSET` target with `source=udp+1000`, UNLESS the
///   derived address equals the advertised one (dedup exact duplicates for
///   the same member).
/// - Members registered ONLY through an advertised QUIC endpoint (no UDP)
///   still get their offset sibling, because the advertised record may be
///   stale (run 38745254834 rejected 2/3 advertised endpoints with alert
///   112) and the offset sibling is the only endpoint class that has ever
///   completed a handshake in recorded runs (`connected_advertised=0`
///   historically).
///
/// Returns `(PeerId, SocketAddr, QuicEndpointSource)` triples.
fn member_dial_targets(
    endpoints: &OverlayMemberEndpoints,
) -> Vec<(PeerId, SocketAddr, QuicEndpointSource)> {
    let seeds = known_member_seeds(endpoints);
    let mut targets = Vec::new();
    for (peer, udp_or_adv) in seeds {
        // Always try the offset sibling for this member's UDP endpoint (or
        // the advertised address that stood in for it).
        let offset_target = SocketAddr::new(
            udp_or_adv.ip(),
            udp_or_adv.port().wrapping_add(QUIC_PORT_OFFSET),
        );

        // If the member has an advertised QUIC endpoint, include it first.
        let advertised = endpoints
            .lock()
            .ok()
            .and_then(|map| map.get(&peer).copied())
            .and_then(|entry| entry.quic);
        if let Some(adv) = advertised {
            targets.push((peer, adv, QuicEndpointSource::Advertised));
            // Dedup: if the derived offset target equals the advertised one,
            // skip adding the duplicate for this member.
            if adv != offset_target {
                targets.push((peer, offset_target, QuicEndpointSource::UdpPortOffset));
            }
        } else {
            // No advertised endpoint — only the offset sibling.
            targets.push((peer, offset_target, QuicEndpointSource::UdpPortOffset));
        }
    }
    targets
}

/// Performs one additional UDP discovery round to widen the member pool,
/// returning the number of newly registered members.
async fn pool_refresh(
    local_addr: SocketAddr,
    keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_bytes: [u8; 32],
    endpoints: OverlayMemberEndpoints,
) -> usize {
    let before = registered_member_count(&endpoints);
    let udp_lookup = udp_overlay_lookup_with_endpoints(
        local_addr,
        *keypair,
        overlay,
        overlay_bytes,
        32,
        Duration::from_secs(30),
        endpoints.clone(),
    );
    let _ =
        tokio::time::timeout(Duration::from_secs(45), udp_lookup(configured_live_seeds())).await;
    let after = registered_member_count(&endpoints);
    after.saturating_sub(before)
}

/// Per-target QUIC connect budget for phase 2b. A single wave measured 0-2
/// handshakes out of 20-35 targets per CI run, so the budget widens the old
/// 10 s window to 15 s to give a slow-but-alive listener time to finish the
/// TLS 1.3 handshake before the dial is written off as a timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum phase 2b waves. Wave 1 dials every registered member's BOTH
/// endpoints (advertised + udp+1000 sibling); waves 2-5 redial only the
/// targets that timed out or errored, with pool refreshes before wave 2 and
/// wave 4 to sample fresh members discovered by additional UDP rounds.
/// Completion is stochastic (shared hosting IPs, transient packet loss, and
/// the set of willing QUIC listeners changes per run), so a single dial is
/// an unreliable sample and the pool must be widened across waves.
const MAX_CONNECT_WAVES: usize = 5;

/// Pause between phase 2b waves, so a redial does not race the previous
/// wave's unanswered handshakes on the same endpoints.
const WAVE_PAUSE: Duration = Duration::from_secs(2);

/// Waves before which we refresh the member pool with an additional UDP
/// discovery round. We refresh before wave 2 (to widen after the first
/// attempt) and before wave 4 (mid-run) so late-joining members get a chance
/// to be dialed.
const POOL_REFRESH_WAVES: &[usize] = &[2, 4];

/// QUIC overlay discovery against the members the UDP path resolves first.
///
/// The bootstrap seeds are `dht-server` deployments, which run no QUIC
/// listener at all (`dht-server/*.cpp` contains no QUIC code), while upstream
/// full nodes bind a `QuicServer` on `adnl_udp_port + 1000`
/// (`quic/quic-sender.h::NODE_PORT_OFFSET`).  Phase 1 therefore runs the
/// proven UDP discovery - twice against one shared
/// [`OverlayMemberEndpoints`] registry when the first round registers fewer
/// than 40 members - to populate the registry; phase 1.5 sends raw Version
/// Negotiation probes to BOTH endpoints of every registered member (advertised
/// `adnl.address.quic` and the derived `udp+1000` sibling) with
/// `1.1.1.1:443` and `8.8.8.8:443` as egress controls, to separate missing
/// listeners from handshake failures; phase 2 probes those members over QUIC
/// and reports the probe counts; phase 2b dials every registered member's
/// BOTH endpoints directly in up to five waves (`CONNECT_TIMEOUT=15s` per
/// target, waves 2-5 redial only the targets that timed out or errored,
/// `WAVE_PAUSE=2s` between waves, stopping early at the first completed
/// handshake). Before wave 2 and wave 4 the member pool is refreshed with an
/// additional UDP discovery round so late-joining members get a chance to be
/// dialed. Every per-target `phase=connect` line carries the
/// `source=advertised` / `source=udp+1000` label of the endpoint that was
/// dialed plus the `wave=` number, and the summary adds
/// `connected_advertised=` / `connected_udp_offset=` counters next to
/// `waves=` and `pool_refreshes=`, so a green or red run shows which endpoint
/// kind answered.
///
/// Soft-skips only the discovery phases - with an explicit
/// `members=`/`quic_peers=` log - when they find nothing and
/// `TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE` is set.  The connect phase
/// hard-requires at least one runtime-discovered member to answer a
/// QUIC handshake, because that is the reachability claim this test
/// exists to check; the gate stays hard by project decision, and the
/// retry waves with pool refresh widen the draw behind it.
#[tokio::test]
#[ignore = "requires live mainnet QUIC access"]
async fn mainnet_overlay_quic_discovery_returns_peers() {
    let _ = pretty_env_logger::try_init();
    let allow_unavailable = std::env::var("TON_MEMPOOL_ALLOW_LIVE_UNAVAILABLE").is_ok();
    let overlay = OverlayId::from_bytes(discovery_overlay_id());
    let local_addr: SocketAddr = "0.0.0.0:0".parse().expect("wildcard address must parse");
    let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let endpoints = OverlayMemberEndpoints::default();

    // Phase 1: ADNL/UDP discovery registers live overlay members and any
    // advertised `adnl.address.quic` their DHT address records carry. One
    // round of `overlay.getRandomPeers` answers reaches only a sample of the
    // overlay (upstream answers with 4 random verified members), so the
    // record budget is twice the old 16 and a sparse first round is followed
    // by a second one against the same registry - `KeyPair` is `Copy`, so
    // the same keypair drives both lookups.
    let udp_lookup = udp_overlay_lookup_with_endpoints(
        local_addr,
        keypair,
        overlay,
        overlay.as_bytes(),
        32,
        Duration::from_secs(30),
        endpoints.clone(),
    );
    let udp_peers: Vec<SeedPeer> =
        tokio::time::timeout(Duration::from_secs(45), udp_lookup(configured_live_seeds()))
            .await
            .unwrap_or_default();
    let mut members = registered_member_count(&endpoints);
    eprintln!(
        "live overlay discovery kind=quic phase=members registered={members} udp_peers={}",
        udp_peers.len()
    );
    if members < 40 {
        let round2_lookup = udp_overlay_lookup_with_endpoints(
            local_addr,
            keypair,
            overlay,
            overlay.as_bytes(),
            32,
            Duration::from_secs(30),
            endpoints.clone(),
        );
        let round2_peers: Vec<SeedPeer> = tokio::time::timeout(
            Duration::from_secs(45),
            round2_lookup(configured_live_seeds()),
        )
        .await
        .unwrap_or_default();
        members = registered_member_count(&endpoints);
        eprintln!(
            "live overlay discovery kind=quic phase=members-round2 registered={members} udp_peers={}",
            round2_peers.len()
        );
    }
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
    // We probe BOTH endpoints per member (advertised + udp+1000 sibling)
    // because the offset sibling is the only class that has ever connected.
    let dial_targets = member_dial_targets(&endpoints);
    let targets: Vec<SocketAddr> = dial_targets.iter().map(|(_, target, _)| *target).collect();
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

    // Phase 2b: direct QUIC connects to the registered members. The lookup
    // above dials members internally but reports only the peers its queries
    // discovered, not which member completed the handshake; dialing each
    // member directly proves that at least one runtime-discovered member -
    // never a configured seed - answers a QUIC connect, and the connected
    // sessions speak the overlay query framing. Handshakes are stochastic -
    // one wave of 20-35 targets answered 0-2 times per recorded CI run - so
    // the dial repeats as waves: wave 1 covers BOTH endpoints of every
    // registered member (advertised + udp+1000 sibling); waves 2-5 redial
    // only the targets that timed out or errored before. Before wave 2 and
    // wave 4 the member pool is refreshed with an additional UDP discovery
    // round so late-joining members get a chance to be dialed. The loop stops
    // as soon as one session is up. Counters are cumulative over all waves,
    // so a target that never answers contributes one timeout (or error) per
    // wave it was dialed in.
    let dial_targets = member_dial_targets(&endpoints);
    let mut pending = dial_targets.clone();
    let mut waves = 0usize;
    let mut connected = 0usize;
    let mut timeouts = 0usize;
    let mut errors = 0usize;
    let mut connected_advertised = 0usize;
    let mut connected_udp_offset = 0usize;
    let mut pool_refreshes = 0usize;
    // Track distinct (peer, target) pairs ever dialed for the summary.
    let mut ever_dialed: std::collections::HashSet<(PeerId, SocketAddr)> =
        dial_targets.iter().map(|(p, t, _)| (*p, *t)).collect();
    while waves < MAX_CONNECT_WAVES && !pending.is_empty() {
        // Pool refresh before wave 2 and wave 4: run an extra UDP discovery
        // round and append any newly registered members' dial targets.
        if POOL_REFRESH_WAVES.contains(&waves.saturating_add(1)) && connected == 0 {
            let new_members = pool_refresh(
                local_addr,
                &keypair,
                overlay,
                overlay.as_bytes(),
                endpoints.clone(),
            )
            .await;
            if new_members > 0 {
                pool_refreshes += 1;
                let fresh_targets = member_dial_targets(&endpoints);
                for (peer, target, source) in fresh_targets {
                    if ever_dialed.insert((peer, target)) {
                        pending.push((peer, target, source));
                    }
                }
                eprintln!(
                    "live overlay discovery kind=quic phase=pool-refresh round={} registered={} new_targets={}",
                    pool_refreshes,
                    registered_member_count(&endpoints),
                    pending.len().saturating_sub(dial_targets.len())
                );
            }
        }
        if waves > 0 {
            tokio::time::sleep(WAVE_PAUSE).await;
        }
        waves += 1;
        let results = join_all(pending.iter().map(|(peer, target, source)| {
            let remote = AdnlPublicKey::from_bytes(peer.as_bytes());
            let (target, source) = (*target, *source);
            async move {
                let Some(remote) = remote else {
                    return (target, source, Err("invalid member public key".to_owned()));
                };
                let attempt = tokio::time::timeout(
                    CONNECT_TIMEOUT,
                    QuicSession::connect(local_addr, target, keypair, remote),
                )
                .await;
                match attempt {
                    Err(_) => (target, source, Err("timeout".to_owned())),
                    Ok(Err(error)) => (target, source, Err(error.to_string())),
                    Ok(Ok(session)) => (target, source, Ok(session)),
                }
            }
        }))
        .await;
        let mut failed: Vec<(PeerId, SocketAddr, QuicEndpointSource)> = Vec::new();
        for (index, (target, source, result)) in results.into_iter().enumerate() {
            let label = source.label();
            match result {
                Ok(session) => {
                    connected += 1;
                    match source {
                        QuicEndpointSource::Advertised => connected_advertised += 1,
                        QuicEndpointSource::UdpPortOffset => connected_udp_offset += 1,
                    }
                    let query = session
                        .overlay_get_random_peers(
                            Int256(overlay.as_bytes()),
                            Duration::from_secs(15),
                        )
                        .await;
                    match query {
                        Ok(nodes) => eprintln!(
                            "live overlay discovery kind=quic phase=connect target={target} source={label} wave={waves} result=connected random_peers={}",
                            nodes.nodes.len()
                        ),
                        Err(error) => eprintln!(
                            "live overlay discovery kind=quic phase=connect target={target} source={label} wave={waves} result=connected query_error={error}"
                        ),
                    }
                }
                Err(error) if error == "timeout" => {
                    timeouts += 1;
                    failed.push(pending[index]);
                    eprintln!(
                        "live overlay discovery kind=quic phase=connect target={target} source={label} wave={waves} result=timeout"
                    );
                }
                Err(error) => {
                    errors += 1;
                    failed.push(pending[index]);
                    eprintln!(
                        "live overlay discovery kind=quic phase=connect target={target} source={label} wave={waves} result=error error={error}"
                    );
                }
            }
        }
        if connected > 0 {
            break;
        }
        pending = failed;
    }
    // Build VN summary for the failure message.
    let vn_summary = format!(
        "VN: targets={} vn_replies={} controls={}",
        member_results.len(),
        vn_replies,
        controls_summary.join(",")
    );
    eprintln!(
        "live overlay discovery kind=quic phase=connect targets={} connected={connected} timeouts={timeouts} errors={errors} connected_advertised={connected_advertised} connected_udp_offset={connected_udp_offset} waves={waves} pool_refreshes={pool_refreshes} distinct_targets={}",
        dial_targets.len(),
        ever_dialed.len()
    );
    assert!(
        connected > 0,
        "live overlay discovery kind=quic phase=connect: no runtime-discovered member answered a QUIC connect ({} targets, {waves} waves, {timeouts} timeouts, {errors} errors, connected_advertised={connected_advertised}, connected_udp_offset={connected_udp_offset}, pool_refreshes={pool_refreshes}); {}; this gate observes mainnet QUIC reachability from the CI runner - a red window is a network-side absence of a QUIC listener accepting this client's handshake, not a client defect; the query framing and TLS profile are pinned by the loopback tests in tonutils-adnl (overlay_get_random_peers_wraps_the_overlay_query_header, quic_session_connects_and_exchanges_query) and documented in docs/reference/network/quic.md",
        ever_dialed.len(),
        vn_summary
    );
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
fn member_dial_targets_yields_both_endpoints() {
    let endpoints = OverlayMemberEndpoints::default();
    let peer = PeerId::from_bytes([1; 32]);
    let udp_addr = "1.2.3.4:54390".parse::<SocketAddr>().unwrap();
    let quic_addr = "1.2.3.4:22222".parse::<SocketAddr>().unwrap();

    // Member with both UDP and advertised QUIC endpoints.
    record_member(&endpoints, peer, udp_addr);
    record_member_quic(&endpoints, peer, quic_addr);
    let targets = member_dial_targets(&endpoints);
    assert_eq!(targets.len(), 2);
    // Both endpoints present with correct sources.
    assert!(
        targets
            .iter()
            .any(|(_, t, s)| *t == quic_addr && *s == QuicEndpointSource::Advertised)
    );
    assert!(targets.iter().any(
        |(_, t, s)| *t == "1.2.3.4:55390".parse::<SocketAddr>().unwrap()
            && *s == QuicEndpointSource::UdpPortOffset
    ));
}

#[test]
fn member_dial_targets_dedup_exact_duplicate() {
    let endpoints = OverlayMemberEndpoints::default();
    let peer = PeerId::from_bytes([2; 32]);
    // Advertised QUIC endpoint equals the derived udp+1000.
    let udp_addr = "1.2.3.4:21222".parse::<SocketAddr>().unwrap();
    let quic_addr = "1.2.3.4:22222".parse::<SocketAddr>().unwrap();

    record_member(&endpoints, peer, udp_addr);
    record_member_quic(&endpoints, peer, quic_addr);
    let targets = member_dial_targets(&endpoints);
    // Should yield only one target (the advertised one) because dedup kicks in.
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].1, quic_addr);
    assert_eq!(targets[0].2, QuicEndpointSource::Advertised);
}

#[test]
fn member_dial_targets_only_udp_no_advertised() {
    let endpoints = OverlayMemberEndpoints::default();
    let peer = PeerId::from_bytes([3; 32]);
    let udp_addr = "1.2.3.4:54390".parse::<SocketAddr>().unwrap();

    record_member(&endpoints, peer, udp_addr);
    let targets = member_dial_targets(&endpoints);
    // Only the offset sibling.
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].1, "1.2.3.4:55390".parse::<SocketAddr>().unwrap());
    assert_eq!(targets[0].2, QuicEndpointSource::UdpPortOffset);
}

#[test]
fn member_dial_targets_only_advertised_no_udp() {
    let endpoints = OverlayMemberEndpoints::default();
    let peer = PeerId::from_bytes([4; 32]);
    let quic_addr = "5.6.7.8:9999".parse::<SocketAddr>().unwrap();

    // Member known ONLY through its advertised QUIC endpoint (no UDP).
    record_member_quic(&endpoints, peer, quic_addr);
    let targets = member_dial_targets(&endpoints);
    // Should yield BOTH the advertised endpoint AND its offset sibling.
    // The offset sibling uses the advertised address as the UDP stand-in
    // (since known_member_seeds surfaces the advertised address as the UDP
    // for such members).
    assert_eq!(targets.len(), 2);
    assert!(
        targets
            .iter()
            .any(|(_, t, s)| *t == quic_addr && *s == QuicEndpointSource::Advertised)
    );
    let offset_from_adv = SocketAddr::new(
        quic_addr.ip(),
        quic_addr.port().wrapping_add(QUIC_PORT_OFFSET),
    );
    assert!(
        targets
            .iter()
            .any(|(_, t, s)| *t == offset_from_adv && *s == QuicEndpointSource::UdpPortOffset)
    );
}
