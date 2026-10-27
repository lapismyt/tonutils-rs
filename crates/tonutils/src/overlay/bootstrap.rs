//! Bootstrap dial helpers, split out of `builder.rs` to keep that file near
//! the repository's 1000 line limit: the concurrent session dial with peer-
//! store recording, the warm-store fallback, the registration wait, and the
//! bootstrap seed resolution helpers.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::overlay::{
    OverlayError, OverlaySessionFactory, PeerManager, ReconnectFactory, SeedPeer,
    discovery_task::DiscoveryRunner,
    peer_store::{PeerStore, SOURCE_BOOTSTRAP},
};

#[cfg(feature = "overlay-config")]
use crate::overlay::PeerId;

use futures::future::join_all;

/// Everything the bootstrap dial needs beyond the peer list.
pub(crate) struct BootstrapDial<'a> {
    pub(crate) factory: &'a OverlaySessionFactory,
    pub(crate) manager: &'a PeerManager,
    pub(crate) reconnect: &'a ReconnectFactory,
    pub(crate) attempts: u32,
    pub(crate) backoff: Duration,
    pub(crate) store: Option<&'a Arc<PeerStore>>,
}

impl BootstrapDial<'_> {
    /// Dials `peers` concurrently through the session factory, registers
    /// every session that answers, records each attempt (handshake latency
    /// included) into the peer store when configured, and returns
    /// `(successes, failures)`.
    pub(crate) async fn dial(&self, peers: &[SeedPeer]) -> (usize, usize) {
        let results = join_all(peers.iter().cloned().map(|peer| {
            let factory = Arc::clone(self.factory);
            let store = self.store.cloned();
            async move {
                log::debug!(
                    "overlay bootstrap: attempting peer={:?} address={}",
                    peer.peer,
                    peer.address
                );
                let started = Instant::now();
                let result = factory(peer.clone()).await;
                if let Some(store) = &store {
                    store
                        .record_dial(
                            peer.peer,
                            &peer.address,
                            SOURCE_BOOTSTRAP,
                            started.elapsed(),
                            result.is_ok(),
                        )
                        .await;
                }
                match &result {
                    Ok(_) => log::debug!(
                        "overlay bootstrap: direct overlay session established for peer={:?} address={}",
                        peer.peer,
                        peer.address
                    ),
                    Err(error) => log::warn!(
                        "overlay bootstrap: peer={:?} address={} failed: {error}",
                        peer.peer,
                        peer.address
                    ),
                }
                result
            }
        }))
        .await;
        let mut successes = 0usize;
        let mut failures = 0usize;
        for result in results {
            match result {
                Ok(session) => {
                    self.manager
                        .add_session_with_reconnect(
                            session,
                            self.reconnect.clone(),
                            self.attempts,
                            self.backoff,
                        )
                        .await;
                    successes += 1;
                }
                Err(error) => {
                    failures += 1;
                    log::warn!("bootstrap session failed: {error}");
                }
            }
        }
        (successes, failures)
    }
}

/// The warm-store retry: when every warm-store dial failed at bootstrap, run
/// the blocking discovery the warm start skipped, dial the fresh members,
/// and keep the stored set in the candidate list so the background rounds can
/// still refresh those peers.  Returns `(successes, failures)`; `(0, 0)` when
/// discovery found nothing to retry with.
pub(crate) async fn warm_store_fallback(
    runner: &DiscoveryRunner,
    seeds: &[SeedPeer],
    pinned: &[SeedPeer],
    saved_all: &[SeedPeer],
    known_peers: &Arc<tokio::sync::RwLock<Vec<SeedPeer>>>,
    dial: &BootstrapDial<'_>,
) -> (usize, usize) {
    let fallback = runner(seeds.to_vec()).await;
    if fallback.is_empty() {
        log::warn!(
            "overlay bootstrap: the peer store produced no sessions and discovery found nothing"
        );
        return (0, 0);
    }
    log::warn!(
        "overlay bootstrap: the peer store produced no sessions, retrying with {} discovered member(s)",
        fallback.len()
    );
    let retry = merge_peer_sets([pinned, fallback.as_slice()]);
    *known_peers.write().await = merge_peer_sets([pinned, fallback.as_slice(), saved_all]);
    let (successes, failures) = dial.dial(&retry).await;
    log::info!(
        "overlay bootstrap: retry factory_successes={successes} factory_failures={failures}"
    );
    (successes, failures)
}

/// Waits until the manager registers at least one peer, bounded by
/// `timeout`; a zero timeout skips the wait entirely.  Shuts the manager
/// down and returns the error when the deadline passes with an empty roster.
pub(crate) async fn await_registered(
    manager: &PeerManager,
    timeout: Duration,
) -> Result<(), OverlayError> {
    if timeout == Duration::ZERO {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + timeout;
    while manager.peer_count().await == 0 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            manager.shutdown();
            return Err(OverlayError::Session(
                "bootstrap sessions did not register any peers".into(),
            ));
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(10))).await;
    }
    log::info!(
        "overlay bootstrap ready: registered_peers={}",
        manager.peer_count().await
    );
    Ok(())
}

/// Merges peer sets in priority order, deduplicated by peer id: an earlier
/// set wins the address for an id carried by several sets (pinned first).
pub(crate) fn merge_peer_sets<'a>(sets: impl IntoIterator<Item = &'a [SeedPeer]>) -> Vec<SeedPeer> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for peer in sets.into_iter().flatten() {
        if seen.insert(peer.peer) {
            merged.push(peer.clone());
        }
    }
    merged
}

#[cfg(feature = "overlay-config")]
pub(crate) async fn download_config(url: &str) -> Result<String, OverlayError> {
    reqwest::get(url)
        .await
        .map_err(|error| OverlayError::ConfigDownload(error.to_string()))?
        .error_for_status()
        .map_err(|error| OverlayError::ConfigDownload(error.to_string()))?
        .text()
        .await
        .map_err(|error| OverlayError::ConfigDownload(error.to_string()))
}

#[cfg(feature = "overlay-config")]
pub(crate) fn parse_seed_json(json: &str) -> Result<Vec<SeedPeer>, OverlayError> {
    let addresses = crate::network_config::extract_dht_addresses(json)
        .map_err(|error| OverlayError::ConfigDownload(error.to_string()))?;
    Ok(addresses
        .into_iter()
        .map(|item| SeedPeer {
            peer: PeerId::from_bytes(item.public_key.unwrap_or([0; 32])),
            address: item.address.to_string(),
        })
        .collect())
}
