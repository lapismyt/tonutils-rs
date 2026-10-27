//! Live diagnostic probe for `overlay.getRandomPeers` answer behaviour.
//!
//! Not part of the CI gate: `#[ignore]`d and run manually against live
//! mainnet members to measure
//!   * how fast a fresh session's first fire-and-forget query is answered,
//!   * whether an exchange right after an answered query is answered,
//!   * whether back-to-back fire-and-forget queries are both answered
//!     (the rate-limit fingerprint),
//!   * whether an exchange after an idle period is answered.
//!
//! Run:
//! `RUST_LOG=tonutils_adnl=trace cargo test -p tonutils-mempool --test live_probe -- --ignored --nocapture`

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use crate::{
    adnl::{AdnlKeyPair, AdnlPublicKey, AdnlUdpSession, AdnlUdpTransport},
    tl::{Int256, adnl::AdnlMessage},
};

/// Mainnet mempool overlay id (the workflow default).
const OVERLAY: Int256 = Int256([
    18, 184, 168, 63, 9, 142, 21, 234, 71, 254, 118, 208, 176, 223, 9, 134, 255, 109, 218, 25, 128,
    121, 107, 8, 75, 13, 42, 104, 178, 85, 134, 73,
]);

/// (label, remote public key, address) pairs captured from a live CI run.
const PROBES: &[(&str, [u8; 32], &str)] = &[
    (
        "peer-173.234.75.230",
        [
            200, 195, 135, 192, 35, 27, 62, 118, 158, 55, 40, 108, 216, 53, 48, 208, 195, 11, 179,
            62, 116, 73, 120, 95, 206, 7, 32, 238, 59, 153, 225, 87,
        ],
        "173.234.75.230:15841",
    ),
    (
        "peer-84.32.98.158-a",
        [
            36, 206, 133, 124, 142, 172, 109, 226, 230, 107, 51, 201, 95, 151, 87, 223, 58, 235,
            40, 150, 61, 11, 38, 121, 34, 75, 147, 132, 108, 4, 23, 45,
        ],
        "84.32.98.158:22809",
    ),
    (
        "peer-84.32.98.158-b",
        [
            182, 129, 105, 13, 112, 100, 152, 207, 146, 141, 108, 12, 117, 96, 59, 152, 87, 213,
            100, 84, 220, 62, 59, 60, 123, 5, 139, 168, 60, 249, 63, 204,
        ],
        "84.32.98.158:47946",
    ),
];

fn stamp(t0: Instant) -> String {
    format!("[{:>8.3}s]", t0.elapsed().as_secs_f64())
}

/// Drains `recv_contents` until `budget` runs out or `stop_after`
/// answers were seen (0 = drain the whole budget).
async fn drain(
    session: &AdnlUdpSession,
    t0: Instant,
    budget: Duration,
    stop_after: usize,
) -> usize {
    let deadline = Instant::now() + budget;
    let mut answers = 0usize;
    loop {
        if stop_after > 0 && answers >= stop_after {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, session.recv_contents()).await {
            Ok(Ok(contents)) => {
                for message in contents
                    .message
                    .iter()
                    .chain(contents.messages.iter().flatten())
                {
                    if let AdnlMessage::Answer { query_id, answer } = message {
                        answers += 1;
                        let hex = query_id.to_hex();
                        println!(
                            "{} ANSWER qid={}.. len={}",
                            stamp(t0),
                            &hex[..12],
                            answer.len()
                        );
                    } else {
                        println!("{} MSG other", stamp(t0));
                    }
                }
            }
            Ok(Err(error)) => {
                println!("{} recv error: {error}", stamp(t0));
                break;
            }
            Err(_) => break,
        }
    }
    answers
}

async fn exchange(session: &AdnlUdpSession, t0: Instant, label: &str) {
    let started = Instant::now();
    match tokio::time::timeout(
        Duration::from_secs(12),
        session.overlay_get_random_peers(OVERLAY, Duration::from_secs(10)),
    )
    .await
    {
        Ok(Ok(nodes)) => println!(
            "{} {label}: OK after {:?} nodes={}",
            stamp(t0),
            started.elapsed(),
            nodes.nodes.len()
        ),
        Ok(Err(error)) => println!(
            "{} {label}: ERR after {:?}: {error}",
            stamp(t0),
            started.elapsed()
        ),
        Err(_) => println!("{} {label}: OUTER TIMEOUT", stamp(t0)),
    }
}

async fn fire_forget(session: &AdnlUdpSession, t0: Instant, label: &str) -> usize {
    println!("{} {label}: fire-and-forget", stamp(t0));
    let _ = session.send_overlay_get_random_peers(OVERLAY).await;
    drain(session, t0, Duration::from_secs(15), 1).await
}

/// Validates the two-step probe protocol the phase and growth will use:
/// on a fresh session, send a fire-and-forget query (this carries the
/// `createChannel` control message and gets no answer), wait for the
/// channel handshake to complete, then run the measured exchange - it
/// must answer in milliseconds.
#[tokio::test]
#[ignore = "live network probe, run manually"]
async fn warmup_then_exchange_answers_on_fresh_session() {
    let _ = pretty_env_logger::try_init();
    let t0 = Instant::now();
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::for_node("0.0.0.0:0".parse().unwrap(), key)
        .await
        .expect("bind transport");
    for (label, peer_bytes, addr) in PROBES {
        let remote = AdnlPublicKey::from_bytes(*peer_bytes).expect("valid ed25519 key");
        let addr: SocketAddr = addr.parse().unwrap();
        let session = transport.session_for_peer(remote, addr);
        println!(
            "{} == {label}: fresh session, warm-up fire-and-forget",
            stamp(t0)
        );
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        tokio::time::sleep(Duration::from_millis(1200)).await;
        exchange(&session, t0, "WARMUP-exchange-1").await;
        exchange(&session, t0, "WARMUP-exchange-2").await;
        drain(&session, t0, Duration::from_secs(1), 0).await;
    }
}

#[tokio::test]
#[ignore = "live network probe, run manually"]
async fn live_member_get_random_peers_behaviour() {
    let _ = pretty_env_logger::try_init();
    let t0 = Instant::now();
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::for_node("0.0.0.0:0".parse().unwrap(), key)
        .await
        .expect("bind transport");
    println!(
        "{} local transport at {:?}",
        stamp(t0),
        transport.local_addr().unwrap()
    );

    for (label, peer_bytes, addr) in PROBES {
        let remote = AdnlPublicKey::from_bytes(*peer_bytes).expect("valid ed25519 key");
        let addr: SocketAddr = addr.parse().unwrap();
        let session = transport.session_for_peer(remote, addr);
        println!("{} == {label} {addr}", stamp(t0));

        // P1: fresh session, first query (like the connect/keepalive shape).
        let n1 = fire_forget(&session, t0, "P1").await;
        println!("{} P1 answers={n1}", stamp(t0));

        // P2: exchange immediately after the answered first query.
        exchange(&session, t0, "P2 exchange-right-after-answer").await;
        drain(&session, t0, Duration::from_secs(3), 0).await;

        // Cooldown reset, then answer -> exchange (the phase's shape).
        println!("{} idle 12s", stamp(t0));
        tokio::time::sleep(Duration::from_secs(12)).await;
        let n2 = fire_forget(&session, t0, "P3").await;
        println!("{} P3 answers={n2}", stamp(t0));
        exchange(&session, t0, "P4 exchange-right-after-answer").await;
        drain(&session, t0, Duration::from_secs(2), 0).await;

        // P5: back-to-back fire-and-forget pair: rate-limit fingerprint.
        println!("{} idle 12s", stamp(t0));
        tokio::time::sleep(Duration::from_secs(12)).await;
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        println!("{} P5 fire-and-forget #1", stamp(t0));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        println!("{} P5 fire-and-forget #2 (1s later)", stamp(t0));
        let n5 = drain(&session, t0, Duration::from_secs(20), 0).await;
        println!(
            "{} P5 answers={n5} (2 => no cooldown, 1 => extras dropped)",
            stamp(t0)
        );

        // P6: exchange after a long idle.
        println!("{} idle 20s", stamp(t0));
        tokio::time::sleep(Duration::from_secs(20)).await;
        exchange(&session, t0, "P6 exchange-after-idle").await;
        drain(&session, t0, Duration::from_secs(3), 0).await;

        // P7: control fire-and-forget after idle.
        let n7 = fire_forget(&session, t0, "P7").await;
        println!("{} P7 answers={n7}", stamp(t0));
    }
}

/// Replicates the CI phase geometry locally: warm sessions driven by the
/// manager's documented receive-loop shape (keepalive gate every 10s plus
/// a 1s `recv_contents` drain, exactly `AdnlUdpOverlaySession::receive`)
/// while exchange probes are issued in waves that straddle the keepalive
/// cadence.
///
/// The CI failure this must reproduce: members answer the keepalive
/// stream but every phase exchange times out (`live.rs:612`), on the
/// same session and within the same windows.  Answers are logged with
/// timestamps from the driver's drain, so a keepalive answer immediately
/// after a timed-out probe - or a late probe answer - is visible
/// directly.
#[tokio::test]
#[ignore = "live network probe, run manually"]
async fn manager_keepalive_vs_probe() {
    let _ = pretty_env_logger::try_init();
    let t0 = Instant::now();
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::for_node("0.0.0.0:0".parse().unwrap(), key)
        .await
        .expect("bind transport");

    let mut sessions = Vec::new();
    for (label, peer_bytes, addr) in PROBES.iter().take(2) {
        let remote = AdnlPublicKey::from_bytes(*peer_bytes).expect("valid ed25519 key");
        let addr: SocketAddr = addr.parse().unwrap();
        let session = transport.session_for_peer(remote, addr);
        // Fresh-session first query: carries createChannel, per the
        // documented warm-up shape.
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        let driver_session = session.clone();
        let driver_label = label.to_string();
        tokio::spawn(async move {
            let mut last_keepalive = Instant::now();
            loop {
                if last_keepalive.elapsed() >= Duration::from_secs(10) {
                    println!("{} [{driver_label}] keepalive sent", stamp(t0));
                    let _ = driver_session.send_overlay_get_random_peers(OVERLAY).await;
                    last_keepalive = Instant::now();
                }
                match tokio::time::timeout(Duration::from_secs(1), driver_session.recv_contents())
                    .await
                {
                    Ok(Ok(contents)) => {
                        for message in contents
                            .message
                            .iter()
                            .chain(contents.messages.iter().flatten())
                        {
                            match message {
                                AdnlMessage::Answer { query_id, answer } => println!(
                                    "{} [{driver_label}] inbound answer qid={}.. len={}",
                                    stamp(t0),
                                    &query_id.to_hex()[..12],
                                    answer.len()
                                ),
                                AdnlMessage::Query { .. } => println!(
                                    "{} [{driver_label}] inbound query (not answered)",
                                    stamp(t0)
                                ),
                                _ => {}
                            }
                        }
                    }
                    Ok(Err(error)) => {
                        println!("{} [{driver_label}] recv error: {error}", stamp(t0));
                        break;
                    }
                    Err(_) => {}
                }
            }
        });
        sessions.push((label.to_string(), session));
    }

    println!(
        "{} warm-up 40s under the manager-like keepalive stream",
        stamp(t0)
    );
    tokio::time::sleep(Duration::from_secs(40)).await;

    // The CI breakage: growth exchanges to the same members succeeded
    // until ~95s of session age and then every waiter-based exchange
    // failed for the rest of the run, while fire-and-forget keepalives
    // stayed answered.  Run waves well past that horizon.
    for round in 1..=10 {
        println!(
            "{} wave-{round}: probing {} sessions",
            stamp(t0),
            sessions.len()
        );
        let probes = sessions.iter().map(|(name, session)| {
            let name = name.clone();
            async move {
                exchange(session, t0, &format!("wave-{round} [{name}]")).await;
            }
        });
        futures::future::join_all(probes).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Exchange with a caller-chosen answer deadline: the growth path uses 5s.
async fn exchange_timed(session: &AdnlUdpSession, t0: Instant, label: &str, deadline: Duration) {
    let started = Instant::now();
    match tokio::time::timeout(
        deadline + Duration::from_secs(2),
        session.overlay_get_random_peers(OVERLAY, deadline),
    )
    .await
    {
        Ok(Ok(nodes)) => println!(
            "{} {label}: OK after {:?} nodes={}",
            stamp(t0),
            started.elapsed(),
            nodes.nodes.len()
        ),
        Ok(Err(error)) => println!(
            "{} {label}: ERR after {:?}: {error}",
            stamp(t0),
            started.elapsed()
        ),
        Err(_) => println!("{} {label}: OUTER TIMEOUT", stamp(t0)),
    }
}

/// Measures the member-side answer budget the CI failure pattern implies:
/// keepalives stay answered while every extra query in the same window is
/// dropped.  Runs a manager-like keepalive stream on the shared session,
/// then (A) lone exchanges, (B) the CI sweep - six concurrent exchanges
/// across the member's own address record (one live port plus five dead),
/// and (C) a keepalive followed 100ms later by an exchange.
#[tokio::test]
#[ignore = "live network probe, run manually"]
async fn member_answer_budget_probe() {
    let _ = pretty_env_logger::try_init();
    let t0 = Instant::now();
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::for_node("0.0.0.0:0".parse().unwrap(), key)
        .await
        .expect("transport binds");
    let (label, peer_bytes, live_addr) = PROBES[0];
    let remote = AdnlPublicKey::from_bytes(peer_bytes).expect("captured key parses");
    let live: SocketAddr = live_addr.parse().unwrap();
    let sweep: Vec<SocketAddr> = [
        "173.234.75.230:48939",
        "173.234.75.230:28840",
        "173.234.75.230:20546",
        "173.234.75.230:54726",
        "173.234.75.230:54771",
    ]
    .iter()
    .map(|addr| addr.parse().unwrap())
    .collect();

    let session = transport.session_for_peer(remote, live);

    let driver = session.clone();
    let driver_label = label.to_string();
    tokio::spawn(async move {
        let mut last_keepalive = Instant::now()
            .checked_sub(Duration::from_secs(11))
            .expect("monotonic clock supports subtraction");
        loop {
            if last_keepalive.elapsed() >= Duration::from_secs(10) {
                let _ = driver.send_overlay_get_random_peers(OVERLAY).await;
                println!("{} [{driver_label}] keepalive sent", stamp(t0));
                last_keepalive = Instant::now();
            }
            match tokio::time::timeout(Duration::from_millis(500), driver.recv_contents()).await {
                Ok(Ok(contents)) => {
                    for message in contents
                        .message
                        .iter()
                        .chain(contents.messages.iter().flatten())
                    {
                        match message {
                            AdnlMessage::Answer { query_id, answer } => println!(
                                "{} [{driver_label}] inbound answer qid={}.. len={}",
                                stamp(t0),
                                &query_id.to_hex()[..12],
                                answer.len()
                            ),
                            AdnlMessage::Query { .. } => println!(
                                "{} [{driver_label}] inbound query (not answered)",
                                stamp(t0)
                            ),
                            _ => {}
                        }
                    }
                }
                Ok(Err(error)) => {
                    println!("{} [{driver_label}] recv error: {error}", stamp(t0));
                    break;
                }
                Err(_) => {}
            }
        }
    });

    println!("{} warm-up 20s under the keepalive stream", stamp(t0));
    tokio::time::sleep(Duration::from_secs(20)).await;

    // A: lone exchange on the live address - the baseline the replica saw
    // answered in tens of milliseconds.
    for round in 1..=4 {
        println!("{} A{round}: lone exchange to {live}", stamp(t0));
        exchange_timed(&session, t0, &format!("A{round}"), Duration::from_secs(5)).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    // B: the CI sweep - one live plus five recorded dead ports of the same
    // member fired concurrently, each through `session_for_peer` so every
    // one adopts its address on the shared session first.
    for round in 1..=4 {
        println!(
            "{} B{round}: sweep of {} addresses",
            stamp(t0),
            sweep.len() + 1
        );
        let probes = std::iter::once(live)
            .chain(sweep.iter().copied())
            .enumerate()
            .map(|(index, addr)| {
                let probe = transport.session_for_peer(remote, addr);
                let label = format!("B{round}.{index}:{addr}");
                async move {
                    exchange_timed(&probe, t0, &label, Duration::from_secs(5)).await;
                }
            });
        futures::future::join_all(probes).await;
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    // C: keepalive then exchange 100ms later, the exact CI overlap of the
    // manager's keepalive with a growth query on one member.
    for round in 1..=3 {
        println!("{} C{round}: keepalive + exchange 100ms later", stamp(t0));
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        exchange_timed(&session, t0, &format!("C{round}"), Duration::from_secs(5)).await;
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    println!("{} budget probe complete", stamp(t0));
    tokio::time::sleep(Duration::from_secs(15)).await;
}

/// Addressed exchange: probes `dest` without touching the session's
/// adopted address.
async fn exchange_to_timed(
    session: &AdnlUdpSession,
    dest: SocketAddr,
    t0: Instant,
    label: &str,
    deadline: Duration,
) {
    let started = Instant::now();
    match tokio::time::timeout(
        deadline + Duration::from_secs(2),
        session.overlay_get_random_peers_to(dest, OVERLAY, deadline),
    )
    .await
    {
        Ok(Ok(nodes)) => println!(
            "{} {label}: OK after {:?} nodes={}",
            stamp(t0),
            started.elapsed(),
            nodes.nodes.len()
        ),
        Ok(Err(error)) => println!(
            "{} {label}: ERR after {:?}: {error}",
            stamp(t0),
            started.elapsed()
        ),
        Err(_) => println!("{} {label}: OUTER TIMEOUT", stamp(t0)),
    }
}

/// The fixed geometry of the CI phase and growth sweeps: the same six
/// concurrent probes across the member's address record, but every one
/// dial-only - the shared session keeps the address the manager adopted
/// and each query carries its own destination.  The live port must
/// answer, the recorded dead ports must time out, the session address
/// must never move, and the manager-like keepalive stream must stay
/// answered for the whole run: before the fix this exact sweep left the
/// session pinned to whichever dead port adopted last, and every later
/// keepalive and exchange was black-holed while the member stayed live.
#[tokio::test]
#[ignore = "live network probe, run manually"]
async fn member_address_sweep_probe() {
    let _ = pretty_env_logger::try_init();
    let t0 = Instant::now();
    let key = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
    let transport = AdnlUdpTransport::for_node("0.0.0.0:0".parse().unwrap(), key)
        .await
        .expect("transport binds");
    let (label, peer_bytes, live_addr) = PROBES[0];
    let remote = AdnlPublicKey::from_bytes(peer_bytes).expect("captured key parses");
    let live: SocketAddr = live_addr.parse().unwrap();
    let sweep: Vec<SocketAddr> = [
        "173.234.75.230:48939",
        "173.234.75.230:28840",
        "173.234.75.230:20546",
        "173.234.75.230:54726",
        "173.234.75.230:54771",
    ]
    .iter()
    .map(|addr| addr.parse().unwrap())
    .collect();

    // The manager establishes the member's address on the shared session.
    let session = transport.session_for_peer(remote, live);

    let driver = session.clone();
    let driver_label = label.to_string();
    tokio::spawn(async move {
        let mut last_keepalive = Instant::now()
            .checked_sub(Duration::from_secs(11))
            .expect("monotonic clock supports subtraction");
        loop {
            if last_keepalive.elapsed() >= Duration::from_secs(10) {
                let _ = driver.send_overlay_get_random_peers(OVERLAY).await;
                println!("{} [{driver_label}] keepalive sent", stamp(t0));
                last_keepalive = Instant::now();
            }
            match tokio::time::timeout(Duration::from_millis(500), driver.recv_contents()).await {
                Ok(Ok(contents)) => {
                    for message in contents
                        .message
                        .iter()
                        .chain(contents.messages.iter().flatten())
                    {
                        match message {
                            AdnlMessage::Answer { query_id, answer } => println!(
                                "{} [{driver_label}] inbound answer qid={}.. len={}",
                                stamp(t0),
                                &query_id.to_hex()[..12],
                                answer.len()
                            ),
                            AdnlMessage::Query { .. } => println!(
                                "{} [{driver_label}] inbound query (not answered)",
                                stamp(t0)
                            ),
                            _ => {}
                        }
                    }
                }
                Ok(Err(error)) => {
                    println!("{} [{driver_label}] recv error: {error}", stamp(t0));
                    break;
                }
                Err(_) => {}
            }
        }
    });

    println!("{} warm-up 20s under the keepalive stream", stamp(t0));
    tokio::time::sleep(Duration::from_secs(20)).await;

    for round in 1..=4 {
        println!(
            "{} S{round}: dial-only sweep of {} addresses",
            stamp(t0),
            sweep.len() + 1
        );
        let probes = std::iter::once(live)
            .chain(sweep.iter().copied())
            .enumerate()
            .map(|(index, addr)| {
                let probe = transport.session_for_probe(remote, addr);
                let label = format!("S{round}.{index}:{addr}");
                async move {
                    exchange_to_timed(&probe, addr, t0, &label, Duration::from_secs(5)).await;
                }
            });
        futures::future::join_all(probes).await;
        println!(
            "{} S{round}: session address after the sweep: {}",
            stamp(t0),
            session.remote_addr()
        );
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    for round in 1..=3 {
        println!(
            "{} C{round}: keepalive + dial-only exchange 100ms later",
            stamp(t0)
        );
        let _ = session.send_overlay_get_random_peers(OVERLAY).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        exchange_to_timed(
            &session,
            live,
            t0,
            &format!("C{round}"),
            Duration::from_secs(5),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(20)).await;
    }

    println!("{} address sweep probe complete", stamp(t0));
    tokio::time::sleep(Duration::from_secs(15)).await;
}
