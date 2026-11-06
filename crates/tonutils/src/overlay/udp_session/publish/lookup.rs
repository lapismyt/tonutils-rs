//! Shared bounded DHT placement and verification primitives.
//!
//! The address and public-membership publishers both need the same
//! iterative `findNode` lookup, the same `k`-closest store set, and the same
//! `findValue` verification walk. Keeping those mechanics here avoids the two
//! publishers drifting apart and makes the walk testable without a live DHT.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use futures::{future::BoxFuture, future::join_all};

use crate::{
    adnl::{
        AdnlKeyPair, AdnlPublicKey, now_i32,
        tl::{
            AdnlNetAddress as Address, AdnlTlPublicKey as TlPublicKey, DhtNode, DhtValue,
            DhtValueResult,
        },
    },
    overlay::SeedPeer,
    tl::TlInt256 as Int256,
};

use super::super::shared_session;

/// Upstream DHT replication factor and lookup width.
pub(super) const DHT_K: usize = 10;

/// The upstream lookup starts from the nearest `2k` candidates.
pub(super) const DHT_LOOKUP_WIDTH: usize = 2 * DHT_K;

/// The same `k` in the signed wire field's width.
const DHT_K_I32: i32 = 10;

/// How many store targets one publish round uses.
pub(super) const STORE_TARGETS: usize = DHT_K;

/// How many candidates one lookup or verification wave asks concurrently.
pub(super) const DHT_ALPHA: usize = 8;

/// Upper bound on lookup and verification waves.
pub(super) const DHT_LOOKUP_WAVES: usize = 6;

/// Per-query deadline for `findNode` and `findValue`.
pub(super) const DHT_QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Per-query deadline for `dht.store`.
pub(super) const DHT_STORE_TIMEOUT: Duration = Duration::from_secs(4);

/// Total deadline for one lookup/store/verification attempt.
pub(super) const DHT_ATTEMPT_BUDGET: Duration = Duration::from_secs(35);

/// Verification keeps the larger of twelve seconds and two fifths of the
/// capped attempt budget, even when the caller passed a shorter timeout.
pub(super) const DHT_VERIFICATION_RESERVE: Duration = Duration::from_secs(14);

/// Upper bound on cached DHT routes held by one publisher.
const DHT_ROUTING_CACHE_CAP: usize = 512;

/// Cached DHT routes older than this are ignored.
const DHT_ROUTING_CACHE_TTL: Duration = Duration::from_mins(10);

/// A DHT node a publish round can query or store a value on.
#[derive(Clone, Debug)]
pub(super) struct DhtCandidate {
    /// The node's position in the DHT keyspace: its ADNL short id.
    pub(super) id: [u8; 32],
    pub(super) public_key: AdnlPublicKey,
    pub(super) address: SocketAddr,
}

#[derive(Clone, Debug)]
struct RoutingCacheEntry {
    candidate: DhtCandidate,
    observed: Instant,
}

#[derive(Debug)]
struct RoutingCacheInner {
    entries: HashMap<[u8; 32], RoutingCacheEntry>,
    capacity: usize,
    ttl: Duration,
}

/// A bounded, expiring cache of DHT routes discovered by one publisher.
///
/// The address and public-membership records of a server share one instance.
/// Only bootstrap seeds and validated `findNode` answers enter this cache;
/// overlay membership records are never routing candidates.
#[derive(Clone, Debug)]
pub(super) struct DhtRoutingCache {
    inner: Arc<Mutex<RoutingCacheInner>>,
}

impl DhtRoutingCache {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RoutingCacheInner {
                entries: HashMap::new(),
                capacity: DHT_ROUTING_CACHE_CAP,
                ttl: DHT_ROUTING_CACHE_TTL,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RoutingCacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn prune(inner: &mut RoutingCacheInner, now: Instant) {
        let ttl = inner.ttl;
        inner
            .entries
            .retain(|_, entry| now.saturating_duration_since(entry.observed) < ttl);
    }

    /// Returns every non-expired route. Callers sort and bound the result for
    /// the key they are currently looking up.
    pub(super) fn candidates(&self, now: Instant) -> Vec<DhtCandidate> {
        let mut inner = self.lock();
        Self::prune(&mut inner, now);
        inner
            .entries
            .values()
            .map(|entry| entry.candidate.clone())
            .collect()
    }

    pub(super) fn observe(&self, candidate: DhtCandidate) {
        self.observe_at(candidate, Instant::now());
    }

    fn observe_at(&self, candidate: DhtCandidate, observed: Instant) {
        let mut inner = self.lock();
        Self::prune(&mut inner, observed);
        if inner.capacity == 0 {
            return;
        }
        if !inner.entries.contains_key(&candidate.id)
            && inner.entries.len() >= inner.capacity
            && let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.observed)
                .map(|(id, _)| *id)
        {
            inner.entries.remove(&oldest);
        }
        inner.entries.insert(
            candidate.id,
            RoutingCacheEntry {
                candidate,
                observed,
            },
        );
    }

    pub(super) fn observe_all(&self, candidates: impl IntoIterator<Item = DhtCandidate>) {
        let now = Instant::now();
        for candidate in candidates {
            self.observe_at(candidate, now);
        }
    }

    #[cfg(test)]
    fn len(&self, now: Instant) -> usize {
        let mut inner = self.lock();
        Self::prune(&mut inner, now);
        inner.entries.len()
    }
}

/// Outcome of one DHT store round.
///
/// `acknowledged` contains only nodes that answered `dht.stored`. That answer
/// confirms the store request was delivered; it does not prove the node kept
/// the value or that `findValue` can serve it. `candidates` is every valid
/// route learned during the attempt and seeds the verification walk; stores
/// themselves use only `LookupOutcome::results`.
#[derive(Debug)]
pub(super) struct PublishAttempt {
    pub(super) acknowledged: Vec<DhtCandidate>,
    pub(super) candidates: Vec<DhtCandidate>,
}

/// One `findNode` query against a candidate.
pub(super) type FindNodeFn = Arc<
    dyn Fn(DhtCandidate, [u8; 32], i32, Duration) -> BoxFuture<'static, Option<Vec<DhtCandidate>>>
        + Send
        + Sync,
>;

/// One `findValue` query against a candidate.
pub(super) type FindValueFn = Arc<
    dyn Fn(DhtCandidate, [u8; 32], i32, Duration) -> BoxFuture<'static, Option<DhtValueResult>>
        + Send
        + Sync,
>;

/// Result of a bounded iterative lookup.
#[derive(Debug)]
pub(super) struct LookupOutcome {
    /// Successful `findNode` responders, closest first and capped at `k`.
    pub(super) results: Vec<DhtCandidate>,
    /// Every valid route learned during the walk, used for verification.
    pub(super) candidates: Vec<DhtCandidate>,
    pub(super) rounds: usize,
    pub(super) asked: usize,
    pub(super) answered: usize,
    pub(super) new_candidates: usize,
    pub(super) stop_reason: &'static str,
}

/// The bounded phases of one lookup/store/verification attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "Each field is a distinct phase deadline and the suffix keeps call sites unambiguous"
)]
pub(super) struct AttemptPhases {
    pub(super) total_deadline: Instant,
    pub(super) publish_deadline: Instant,
    pub(super) lookup_deadline: Instant,
    pub(super) verification_deadline: Instant,
}

/// Result of a bounded `findValue` verification walk.
#[derive(Debug)]
pub(super) struct VerificationOutcome {
    pub(super) served: bool,
    pub(super) rounds: usize,
    pub(super) asked: usize,
    pub(super) answered: usize,
    pub(super) new_candidates: usize,
    pub(super) stale_values: usize,
    pub(super) stop_reason: &'static str,
}

/// Splits one attempt into publish, lookup, and verification phases.
pub(super) fn attempt_phases(timeout: Duration) -> AttemptPhases {
    attempt_phases_at(Instant::now(), timeout)
}

fn attempt_phases_at(start: Instant, timeout: Duration) -> AttemptPhases {
    let budget = timeout.min(DHT_ATTEMPT_BUDGET);
    let total_deadline = start + budget;
    let verification_reserve = DHT_VERIFICATION_RESERVE.min(budget);
    let publish_deadline = total_deadline
        .checked_sub(verification_reserve)
        .unwrap_or(start);
    let lookup_deadline = publish_deadline
        .checked_sub(DHT_STORE_TIMEOUT)
        .unwrap_or(start);
    AttemptPhases {
        total_deadline,
        publish_deadline,
        lookup_deadline,
        verification_deadline: total_deadline,
    }
}

/// XOR distance between two DHT key ids - the ordering the DHT
/// uses to pick the nodes responsible for a key.
pub(super) fn xor_distance(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut distance = [0u8; 32];
    for i in 0..32 {
        distance[i] = a[i] ^ b[i];
    }
    distance
}

/// Sorts and deduplicates candidates by DHT node id, closest first.
pub(super) fn sort_candidates(candidates: &mut Vec<DhtCandidate>, key_id: &[u8; 32]) {
    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.id));
    candidates.sort_by_key(|candidate| xor_distance(&candidate.id, key_id));
}

fn insert_candidate(candidates: &mut Vec<DhtCandidate>, candidate: DhtCandidate) -> bool {
    if candidates
        .iter()
        .any(|existing| existing.id == candidate.id)
    {
        return false;
    }
    candidates.push(candidate);
    true
}

fn insert_pending(
    pending: &mut Vec<DhtCandidate>,
    tried: &HashSet<[u8; 32]>,
    key_id: &[u8; 32],
    candidate: DhtCandidate,
) -> bool {
    if tried.contains(&candidate.id) || !insert_candidate(pending, candidate) {
        return false;
    }
    sort_candidates(pending, key_id);
    pending.truncate(DHT_LOOKUP_WIDTH);
    true
}

fn insert_result(results: &mut Vec<DhtCandidate>, key_id: &[u8; 32], candidate: DhtCandidate) {
    let _ = insert_candidate(results, candidate);
    sort_candidates(results, key_id);
    results.truncate(DHT_K);
}

/// Returns the closest not-yet-tried candidates for one verification wave,
/// querying acknowledged store targets before the wider candidate set.
pub(super) fn next_verification_batch(
    candidates: &[DhtCandidate],
    key_id: &[u8; 32],
    tried: &HashSet<[u8; 32]>,
    priority: &HashSet<[u8; 32]>,
) -> Vec<DhtCandidate> {
    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|&index| !tried.contains(&candidates[index].id))
        .collect();
    order.sort_by_key(|&index| {
        let candidate = &candidates[index];
        (
            !priority.contains(&candidate.id),
            xor_distance(&candidate.id, key_id),
        )
    });
    order
        .into_iter()
        .take(DHT_ALPHA)
        .map(|index| candidates[index].clone())
        .collect()
}

/// The publish round's starting candidate set: the known bootstrap DHT
/// resolver nodes. Overlay members are never inserted here.
pub(super) fn known_candidates(dht_nodes: &[SeedPeer]) -> Vec<DhtCandidate> {
    let mut candidates = Vec::with_capacity(dht_nodes.len());
    for node in dht_nodes {
        let Ok(address) = node.address.parse() else {
            continue;
        };
        let Some(public_key) = AdnlPublicKey::from_bytes(node.peer.as_bytes()) else {
            continue;
        };
        let candidate = DhtCandidate {
            id: short_id(&public_key),
            public_key,
            address,
        };
        let _ = insert_candidate(&mut candidates, candidate);
    }
    candidates
}

/// The nearest `2k` bootstrap and cached routes that seed one lookup.
///
/// Seeds are refreshed in the shared cache on every attempt. Overlay members
/// are dial targets only and cannot enter this set.
pub(super) fn initial_lookup_candidates(
    cache: &DhtRoutingCache,
    dht_nodes: &[SeedPeer],
    key_id: &[u8; 32],
    now: Instant,
) -> Vec<DhtCandidate> {
    let mut candidates = known_candidates(dht_nodes);
    cache.observe_all(candidates.iter().cloned());
    candidates.extend(cache.candidates(now));
    sort_candidates(&mut candidates, key_id);
    candidates.truncate(DHT_LOOKUP_WIDTH);
    candidates
}

/// The full verification seed set: bootstrap resolvers, enriched lookup
/// candidates, cached routes, and nodes that acknowledged `dht.store`,
/// deduplicated by DHT node id and sorted by distance to the key.
pub(super) fn verification_candidates(
    key_id: &[u8; 32],
    dht_nodes: &[SeedPeer],
    publish_candidates: &[DhtCandidate],
    acknowledged_targets: &[DhtCandidate],
    cache: &DhtRoutingCache,
    now: Instant,
) -> Vec<DhtCandidate> {
    let mut pending = known_candidates(dht_nodes);
    for candidate in publish_candidates
        .iter()
        .chain(acknowledged_targets)
        .chain(cache.candidates(now).iter())
    {
        let _ = insert_candidate(&mut pending, candidate.clone());
    }
    sort_candidates(&mut pending, key_id);
    pending
}

/// Selects the closest store targets. Callers pass only successful `findNode`
/// responders, so a short result set stays short rather than being padded.
pub(super) fn store_targets(candidates: &[DhtCandidate], key_id: &[u8; 32]) -> Vec<DhtCandidate> {
    let mut selected = candidates.to_vec();
    sort_candidates(&mut selected, key_id);
    selected.truncate(STORE_TARGETS);
    selected
}

/// Merges one retry round into the accumulated placement evidence.
pub(super) fn merge_attempt(
    acknowledged: &mut Vec<DhtCandidate>,
    candidates: &mut Vec<DhtCandidate>,
    attempt: PublishAttempt,
) {
    for candidate in attempt.acknowledged {
        let _ = insert_candidate(acknowledged, candidate);
    }
    for candidate in attempt.candidates {
        let _ = insert_candidate(candidates, candidate);
    }
}

/// Returns the closest not-yet-tried candidates for one lookup wave.
pub(super) fn next_lookup_batch(
    candidates: &[DhtCandidate],
    key_id: &[u8; 32],
    tried: &HashSet<[u8; 32]>,
) -> Vec<DhtCandidate> {
    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|&index| !tried.contains(&candidates[index].id))
        .collect();
    order.sort_by_key(|&index| xor_distance(&candidates[index].id, key_id));
    order
        .into_iter()
        .take(DHT_ALPHA)
        .map(|index| candidates[index].clone())
        .collect()
}

/// Extracts a publish candidate from a `dht.node` record.
pub(super) fn candidate_from_dht_node(node: &DhtNode) -> Option<DhtCandidate> {
    let TlPublicKey::Ed25519 { key } = &node.id else {
        return None;
    };
    let public_key = AdnlPublicKey::from_bytes(key.0)?;
    let Address::Udp { ip, port } = node.addr_list.addrs.first()? else {
        return None;
    };
    if *port <= 0 || *ip == 0 {
        return None;
    }
    let address = SocketAddr::new(
        std::net::Ipv4Addr::from(ip.cast_unsigned()).into(),
        u16::try_from(*port).ok()?,
    );
    Some(DhtCandidate {
        id: short_id(&public_key),
        public_key,
        address,
    })
}

/// Runs a bounded iterative `findNode` lookup and returns the merged,
/// distance-sorted placement evidence.
///
/// `pending` holds at most the nearest `2k` untried candidates, while
/// `results` holds only successful responders and is capped at `k`. The walk
/// converges only after ten peers answered and no pending candidate is closer
/// than the tenth successful result. Discovery alone never completes it.
#[expect(
    clippy::too_many_lines,
    reason = "The lookup state machine keeps response handling, convergence, and counters together"
)]
pub(super) async fn lookup_closest(
    operation: &'static str,
    attempt: usize,
    mut pending: Vec<DhtCandidate>,
    key_id: [u8; 32],
    deadline: Instant,
    cache: &DhtRoutingCache,
    query: FindNodeFn,
) -> LookupOutcome {
    let initial_candidates = pending.len();
    let mut learned = pending.clone();
    sort_candidates(&mut pending, &key_id);
    pending.truncate(DHT_LOOKUP_WIDTH);
    let mut tried = HashSet::new();
    let mut results: Vec<DhtCandidate> = Vec::with_capacity(DHT_K);
    let mut rounds = 0usize;
    let mut asked = 0usize;
    let mut answered = 0usize;
    let mut new_candidates = 0usize;
    let mut stop_reason = "wave_limit";

    for wave in 1..=DHT_LOOKUP_WAVES {
        pending.retain(|candidate| !tried.contains(&candidate.id));
        sort_candidates(&mut pending, &key_id);
        pending.truncate(DHT_LOOKUP_WIDTH);

        if results.len() == DHT_K {
            let kth_distance = xor_distance(&results[DHT_K - 1].id, &key_id);
            let nearest_pending = pending
                .first()
                .map(|candidate| xor_distance(&candidate.id, &key_id));
            if nearest_pending.is_none_or(|distance| distance >= kth_distance) {
                stop_reason = "converged";
                break;
            }
        }

        let batch = next_lookup_batch(&pending, &key_id, &tried);
        if batch.is_empty() {
            stop_reason = if results.len() == DHT_K {
                "converged"
            } else {
                "candidate_exhausted"
            };
            break;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            stop_reason = "deadline";
            break;
        }
        rounds = wave;
        let timeout = DHT_QUERY_TIMEOUT.min(remaining);
        for candidate in &batch {
            tried.insert(candidate.id);
        }
        pending.retain(|candidate| !tried.contains(&candidate.id));
        let batch_len = batch.len();
        asked += batch_len;

        let responses = join_all(batch.into_iter().map(|candidate| {
            let query = Arc::clone(&query);
            async move {
                let result = query(candidate.clone(), key_id, DHT_K_I32, timeout).await;
                (candidate, result)
            }
        }))
        .await;

        let mut wave_answered = 0usize;
        let mut wave_new = 0usize;
        for (candidate, result) in responses {
            let Some(nodes) = result else {
                continue;
            };
            wave_answered += 1;
            cache.observe(candidate.clone());
            let _ = insert_candidate(&mut learned, candidate.clone());
            log::debug!(
                "{operation}: findNode via {} answered on wave {wave}",
                candidate.address
            );
            insert_result(&mut results, &key_id, candidate);
            for node in nodes {
                cache.observe(node.clone());
                if insert_candidate(&mut learned, node.clone()) {
                    wave_new += 1;
                }
                let _ = insert_pending(&mut pending, &tried, &key_id, node);
            }
        }
        answered += wave_answered;
        new_candidates += wave_new;
        log::debug!(
            "{operation}: lookup attempt={attempt} wave={wave} asked={batch_len} answered={wave_answered} new_candidates={wave_new} pending={} results={}",
            pending.len(),
            results.len()
        );
    }

    sort_candidates(&mut learned, &key_id);
    sort_candidates(&mut results, &key_id);
    log::debug!(
        "{operation}: lookup attempt={attempt} initial_candidates={initial_candidates} pending={} rounds={rounds} asked={asked} answered={answered} new_candidates={new_candidates} successful_results={} candidates={} stop={stop_reason}",
        pending.len(),
        results.len(),
        learned.len()
    );
    LookupOutcome {
        results,
        candidates: learned,
        rounds,
        asked,
        answered,
        new_candidates,
        stop_reason,
    }
}

/// Verifies a value through a bounded `findValue` walk.
///
/// A stale `valueFound` means the peer answered, but it is not the value this
/// node just published. In that case the walk asks the same peer for
/// `findNode` and follows the returned nodes. `valueNotFound` nodes are merged
/// the same way. Success is only reported when `matches` accepts a served
/// value.
#[expect(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "One verification wave must keep answer handling, stale follow-up, and counters together"
)]
pub(super) async fn verify_value<F>(
    operation: &'static str,
    mut pending: Vec<DhtCandidate>,
    key_id: [u8; 32],
    deadline: Instant,
    find_value: FindValueFn,
    find_node: FindNodeFn,
    priority: &HashSet<[u8; 32]>,
    cache: &DhtRoutingCache,
    matches: F,
) -> VerificationOutcome
where
    F: Fn(&DhtValue) -> bool + Send + Sync,
{
    let mut tried = HashSet::new();
    let mut rounds = 0usize;
    let mut asked = 0usize;
    let mut answered = 0usize;
    let mut new_candidates = 0usize;
    let mut stale_values = 0usize;
    let mut stop_reason = "wave_limit";

    for wave in 1..=DHT_LOOKUP_WAVES {
        sort_candidates(&mut pending, &key_id);
        let batch = next_verification_batch(&pending, &key_id, &tried, priority);
        if batch.is_empty() {
            stop_reason = "candidate_exhausted";
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            stop_reason = "deadline";
            break;
        }
        rounds = wave;
        let timeout = DHT_QUERY_TIMEOUT.min(remaining);
        for candidate in &batch {
            tried.insert(candidate.id);
        }
        let batch_len = batch.len();
        let priority_asked = batch
            .iter()
            .filter(|candidate| priority.contains(&candidate.id))
            .count();
        asked += batch_len;

        let responses = join_all(batch.into_iter().map(|candidate| {
            let find_value = Arc::clone(&find_value);
            async move {
                let result = find_value(candidate.clone(), key_id, DHT_K_I32, timeout).await;
                (candidate, result)
            }
        }))
        .await;

        let mut wave_answered = 0usize;
        let mut wave_new = 0usize;
        let mut stale_peers = Vec::new();
        for (candidate, result) in responses {
            let Some(result) = result else {
                continue;
            };
            wave_answered += 1;
            cache.observe(candidate.clone());
            match result {
                DhtValueResult::Found { value } => {
                    if matches(&value) {
                        log::debug!(
                            "{operation}: served on {operation} node={} wave={wave}",
                            candidate.address
                        );
                        return VerificationOutcome {
                            served: true,
                            rounds,
                            asked,
                            answered: answered + wave_answered,
                            new_candidates,
                            stale_values,
                            stop_reason: "served",
                        };
                    }
                    stale_values += 1;
                    log::debug!(
                        "{operation}: stale valueFound via {} on wave {wave}; following findNode",
                        candidate.address
                    );
                    stale_peers.push(candidate);
                }
                DhtValueResult::NotFound { nodes } => {
                    let now = now_i32();
                    for node in nodes.nodes {
                        if !node.is_valid(now) {
                            continue;
                        }
                        if let Some(closer) = candidate_from_dht_node(&node) {
                            cache.observe(closer.clone());
                            if insert_candidate(&mut pending, closer) {
                                wave_new += 1;
                            }
                        }
                    }
                }
            }
        }
        answered += wave_answered;

        let follow_remaining = deadline.saturating_duration_since(Instant::now());
        if !stale_peers.is_empty() && !follow_remaining.is_zero() {
            let follow_timeout = DHT_QUERY_TIMEOUT.min(follow_remaining);
            let follow = join_all(stale_peers.into_iter().map(|candidate| {
                let find_node = Arc::clone(&find_node);
                async move {
                    let nodes = find_node(candidate, key_id, DHT_K_I32, follow_timeout).await;
                    nodes.unwrap_or_default()
                }
            }))
            .await;
            for nodes in follow {
                for node in nodes {
                    cache.observe(node.clone());
                    if insert_candidate(&mut pending, node) {
                        wave_new += 1;
                    }
                }
            }
        }
        new_candidates += wave_new;
        log::debug!(
            "{operation}: wave={wave} asked={batch_len} priority_asked={priority_asked} answered={wave_answered} new_candidates={wave_new} stale_values={stale_values} pending={}",
            pending.len()
        );
    }

    log::debug!(
        "{operation}: rounds={rounds} asked={asked} answered={answered} new_candidates={new_candidates} stale_values={stale_values} stop={stop_reason}"
    );
    VerificationOutcome {
        served: false,
        rounds,
        asked,
        answered,
        new_candidates,
        stale_values,
        stop_reason,
    }
}

/// Builds a production `findNode` requester for one local identity.
pub(super) fn find_node_requester(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
) -> FindNodeFn {
    let local_keypair = *local_keypair;
    Arc::new(move |candidate, key_id, k, timeout| {
        Box::pin(async move {
            tokio::time::timeout(timeout, async move {
                let session = shared_session(
                    local_addr,
                    &local_keypair,
                    candidate.public_key,
                    candidate.address,
                )
                .await
                .ok()?;
                let nodes = session
                    .dht_find_node(Int256(key_id), k, timeout)
                    .await
                    .ok()?;
                let now = now_i32();
                Some(
                    nodes
                        .nodes
                        .into_iter()
                        .filter(|node| node.is_valid(now))
                        .filter_map(|node| candidate_from_dht_node(&node))
                        .collect(),
                )
            })
            .await
            .ok()
            .flatten()
        })
    })
}

/// Builds a production `findValue` requester for one local identity.
pub(super) fn find_value_requester(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
) -> FindValueFn {
    let local_keypair = *local_keypair;
    Arc::new(move |candidate, key_id, k, timeout| {
        Box::pin(async move {
            tokio::time::timeout(timeout, async move {
                let session = shared_session(
                    local_addr,
                    &local_keypair,
                    candidate.public_key,
                    candidate.address,
                )
                .await
                .ok()?;
                session
                    .dht_find_value(Int256(key_id), k, timeout)
                    .await
                    .ok()
            })
            .await
            .ok()
            .flatten()
        })
    })
}

/// The ADNL short id of a public key: the node's position in the DHT
/// keyspace.
pub(super) fn short_id(public_key: &AdnlPublicKey) -> [u8; 32] {
    crate::adnl::AdnlAddress::from(public_key).to_bytes()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use ed25519_dalek::{Signer, SigningKey};

    use crate::{
        adnl::{
            AdnlKeyPair, AdnlPublicKey, now_i32,
            tl::{
                AddressList, AdnlNetAddress, AdnlTlPublicKey as TlPublicKey, DhtKey,
                DhtKeyDescription, DhtNode, DhtNodeBoxed, DhtNodes, DhtUpdateRule, DhtValue,
                DhtValueResult,
            },
        },
        tl::TlInt256 as Int256,
    };

    use super::{
        DHT_ALPHA, DHT_K, DHT_LOOKUP_WIDTH, DHT_ROUTING_CACHE_TTL, DHT_STORE_TIMEOUT,
        DHT_VERIFICATION_RESERVE, DhtCandidate, DhtRoutingCache, FindNodeFn, FindValueFn,
        STORE_TARGETS, attempt_phases_at, candidate_from_dht_node, initial_lookup_candidates,
        lookup_closest, next_lookup_batch, next_verification_batch, short_id, store_targets,
        verification_candidates, verify_value,
    };

    fn candidate(id: u8, port: u16) -> DhtCandidate {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        DhtCandidate {
            id: [id; 32],
            public_key: keypair.public_key,
            address: format!("127.0.0.1:{port}").parse().unwrap(),
        }
    }

    fn dummy_value(tag: u8) -> DhtValue {
        DhtValue {
            key: DhtKeyDescription {
                key: DhtKey {
                    id: Int256([0; 32]),
                    name: b"test".to_vec(),
                    idx: 0,
                },
                id: TlPublicKey::Ed25519 {
                    key: Int256([0; 32]),
                },
                update_rule: DhtUpdateRule::Anybody,
                signature: Vec::new(),
            },
            value: vec![tag],
            ttl: now_i32().saturating_add(60),
            signature: Vec::new(),
        }
    }

    fn empty_not_found() -> DhtValueResult {
        DhtValueResult::NotFound {
            nodes: DhtNodes { nodes: Vec::new() },
        }
    }

    fn signed_dht_node(seed: u8, port: u16) -> (DhtNode, DhtCandidate) {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let public_key = AdnlPublicKey::from_bytes(key.verifying_key().to_bytes()).unwrap();
        let node = DhtNode {
            id: TlPublicKey::Ed25519 {
                key: Int256(key.verifying_key().to_bytes()),
            },
            addr_list: AddressList {
                addrs: vec![AdnlNetAddress::Udp {
                    ip: 0x7f000001,
                    port: i32::from(port),
                }],
                version: 1,
                reinit_date: 1,
                priority: 0,
                expire_at: 0,
            },
            version: now_i32(),
            signature: Vec::new(),
        };
        let unsigned = DhtNodeBoxed {
            id: node.id.clone(),
            addr_list: node.addr_list.clone(),
            version: node.version,
            signature: Vec::new(),
        };
        let mut signed = node;
        signed.signature = key.sign(&tl_proto::serialize(unsigned)).to_bytes().to_vec();
        let candidate = DhtCandidate {
            id: short_id(&public_key),
            public_key,
            address: format!("127.0.0.1:{port}").parse().unwrap(),
        };
        (signed, candidate)
    }

    #[tokio::test]
    async fn lookup_converges_to_ten_closest() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let seed = candidate(200, 40_000);
        let discovered: Vec<DhtCandidate> = (1..=12)
            .map(|id| candidate(id, 40_000 + u16::from(id)))
            .collect();

        let mut topology = HashMap::new();
        topology.insert(seed.id, discovered);
        let topology = Arc::new(topology);
        let query: FindNodeFn = Arc::new(move |candidate, _, _, _| {
            let nodes = topology.get(&candidate.id).cloned().unwrap_or_default();
            Box::pin(async move { Some(nodes) })
        });

        let outcome = lookup_closest(
            "test lookup",
            0,
            vec![seed],
            key_id,
            Instant::now() + Duration::from_secs(5),
            &cache,
            query,
        )
        .await;

        assert!(outcome.candidates.len() >= DHT_K);
        assert_eq!(outcome.results.len(), DHT_K);
        let ids: Vec<u8> = outcome
            .results
            .iter()
            .map(|candidate| candidate.id[0])
            .collect();
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        assert_eq!(outcome.stop_reason, "converged");
    }

    #[test]
    fn lookup_starts_from_the_nearest_twenty_routes() {
        let key_id = [0u8; 32];
        let now = Instant::now();
        let cache = DhtRoutingCache::new();
        for id in 1..=25 {
            cache.observe_at(candidate(id, 40_000 + u16::from(id)), now);
        }

        let initial = initial_lookup_candidates(&cache, &[], &key_id, now);

        assert_eq!(initial.len(), DHT_LOOKUP_WIDTH);
        assert_eq!(
            initial
                .iter()
                .map(|candidate| candidate.id[0])
                .collect::<Vec<_>>(),
            (1..=20).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn lookup_does_not_converge_on_discovery_alone() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let seed = candidate(200, 40_100);
        let seed_id = seed.id;
        let discovered: Vec<DhtCandidate> = (1..=12)
            .map(|id| candidate(id, 40_100 + u16::from(id)))
            .collect();
        let query: FindNodeFn = Arc::new(move |candidate, _, _, _| {
            let nodes = if candidate.id == seed_id {
                Some(discovered.clone())
            } else {
                None
            };
            Box::pin(async move { nodes })
        });

        let outcome = lookup_closest(
            "test lookup",
            0,
            vec![seed.clone()],
            key_id,
            Instant::now() + Duration::from_secs(5),
            &cache,
            query,
        )
        .await;

        assert_eq!(outcome.results.len(), 1);
        assert_eq!(outcome.results[0].id, seed.id);
        assert_eq!(outcome.stop_reason, "candidate_exhausted");
    }

    #[test]
    fn attempt_phases_reserve_a_real_verification_window() {
        let start = Instant::now();
        let phases = attempt_phases_at(start, Duration::from_secs(30));

        assert_eq!(phases.total_deadline, start + Duration::from_secs(30));
        assert!(
            phases
                .verification_deadline
                .saturating_duration_since(phases.publish_deadline)
                >= Duration::from_secs(12)
        );
        assert_eq!(
            phases
                .publish_deadline
                .saturating_duration_since(phases.lookup_deadline),
            DHT_STORE_TIMEOUT
        );
        assert!(phases.lookup_deadline > start);

        let capped = attempt_phases_at(start, Duration::from_secs(60));
        assert_eq!(capped.total_deadline, start + Duration::from_secs(35));
        assert!(
            capped
                .verification_deadline
                .saturating_duration_since(capped.publish_deadline)
                >= DHT_VERIFICATION_RESERVE
        );
    }

    #[tokio::test]
    async fn lookup_stops_when_its_deadline_is_already_expired() {
        let cache = DhtRoutingCache::new();
        let query: FindNodeFn = Arc::new(|_, _, _, _| Box::pin(async { None }));

        let outcome = lookup_closest(
            "test lookup",
            0,
            vec![candidate(1, 40_300)],
            [0u8; 32],
            Instant::now(),
            &cache,
            query,
        )
        .await;

        assert_eq!(outcome.stop_reason, "deadline");
        assert_eq!(outcome.asked, 0);
    }

    #[test]
    fn store_uses_exactly_ten_closest_targets() {
        let key_id = [0u8; 32];
        let candidates: Vec<DhtCandidate> = (1..=14)
            .map(|id| candidate(id, 41_000 + u16::from(id)))
            .collect();

        let targets = store_targets(&candidates, &key_id);
        assert!(candidates.len() > STORE_TARGETS);
        assert_eq!(targets.len(), STORE_TARGETS);
        assert_eq!(
            targets
                .iter()
                .map(|candidate| candidate.id[0])
                .collect::<Vec<_>>(),
            (1..=10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn store_targets_do_not_pad_beyond_successful_responders() {
        let responders = vec![
            candidate(7, 41_100),
            candidate(1, 41_101),
            candidate(4, 41_102),
        ];
        let targets = store_targets(&responders, &[0u8; 32]);

        assert_eq!(targets.len(), responders.len());
        assert_eq!(
            targets
                .iter()
                .map(|candidate| candidate.id[0])
                .collect::<Vec<_>>(),
            vec![1, 4, 7]
        );
    }

    #[tokio::test]
    async fn store_targets_exclude_discovered_non_responders() {
        let cache = DhtRoutingCache::new();
        let seed = candidate(200, 41_200);
        let seed_id = seed.id;
        let discovered: Vec<DhtCandidate> = (1..=12)
            .map(|id| candidate(id, 41_200 + u16::from(id)))
            .collect();
        let query: FindNodeFn = Arc::new(move |asked, _, _, _| {
            let result = if asked.id == seed_id {
                Some(discovered.clone())
            } else if asked.id[0] <= 6 {
                Some(Vec::new())
            } else {
                None
            };
            Box::pin(async move { result })
        });

        let outcome = lookup_closest(
            "test lookup",
            0,
            vec![seed],
            [0u8; 32],
            Instant::now() + Duration::from_secs(5),
            &cache,
            query,
        )
        .await;
        let targets = store_targets(&outcome.results, &[0u8; 32]);
        let target_ids: HashSet<[u8; 32]> = targets.iter().map(|target| target.id).collect();

        assert_eq!(outcome.results.len(), 7);
        assert_eq!(targets.len(), 7);
        assert!((7..=12).all(|id| !target_ids.contains(&[id; 32])));
    }

    #[test]
    fn routing_cache_persists_across_clones_and_deduplicates_by_node_id() {
        let now = Instant::now();
        let cache = DhtRoutingCache::new();
        cache.observe_at(candidate(1, 42_100), now);
        let shared = cache.clone();
        cache.observe_at(candidate(1, 42_101), now);

        let candidates = shared.candidates(now);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].address.port(), 42_101);
    }

    #[test]
    fn routing_cache_expires_and_enforces_its_capacity() {
        let now = Instant::now();
        let cache = DhtRoutingCache::new();
        cache.observe_at(candidate(1, 42_200), now);
        assert_eq!(
            cache.len(now + DHT_ROUTING_CACHE_TTL.saturating_sub(Duration::from_millis(1))),
            1
        );
        assert_eq!(cache.len(now + DHT_ROUTING_CACHE_TTL), 0);

        {
            let mut inner = cache.lock();
            inner.capacity = 2;
        }
        cache.observe_at(candidate(2, 42_201), now);
        cache.observe_at(candidate(3, 42_202), now + Duration::from_secs(1));
        cache.observe_at(candidate(4, 42_203), now + Duration::from_secs(2));
        let mut ids: Vec<u8> = cache
            .candidates(now + Duration::from_secs(3))
            .iter()
            .map(|candidate| candidate.id[0])
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![3, 4]);
    }

    #[test]
    fn routing_cache_never_accepts_overlay_membership_nodes() {
        use crate::overlay::tl::{OverlayNode, OverlayNodesBoxed};

        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let overlay_node = OverlayNode {
            id: TlPublicKey::Ed25519 {
                key: Int256(keypair.public_key.to_bytes()),
            },
            overlay: Int256([9; 32]),
            version: now_i32(),
            signature: Vec::new(),
        };
        let membership = tl_proto::serialize(OverlayNodesBoxed {
            nodes: vec![overlay_node.clone()],
        });
        assert!(tl_proto::deserialize::<DhtNode>(&tl_proto::serialize(&overlay_node)).is_err());
        assert!(tl_proto::deserialize::<DhtNode>(&membership).is_err());

        let cache = DhtRoutingCache::new();
        assert_eq!(cache.len(Instant::now()), 0);
    }

    #[tokio::test]
    async fn acknowledged_store_is_not_success_without_a_served_value() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let a = candidate(1, 42_001);
        let now = Instant::now();
        let pending = verification_candidates(
            &key_id,
            &[],
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
            &cache,
            now,
        );
        let find_value_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&find_value_calls);
        let find_value: FindValueFn = Arc::new(move |_, _, _, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Some(empty_not_found()) })
        });
        let find_node: FindNodeFn = Arc::new(|_, _, _, _| Box::pin(async { None }));
        let priority = HashSet::new();

        let outcome = verify_value(
            "test verify",
            pending,
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            &priority,
            &cache,
            |_: &DhtValue| false,
        )
        .await;

        assert!(!outcome.served);
        assert_eq!(find_value_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stale_value_found_triggers_find_node_and_then_serves() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let a = candidate(1, 43_001);
        let b = candidate(2, 43_002);
        let stale = dummy_value(0);
        let fresh = dummy_value(1);

        let find_node_calls = Arc::new(AtomicUsize::new(0));
        let find_node_calls_inner = Arc::clone(&find_node_calls);
        let find_node: FindNodeFn = Arc::new(move |asked, _, k, _| {
            assert_eq!(k, 10);
            find_node_calls_inner.fetch_add(1, Ordering::Relaxed);
            assert_eq!(asked.id, a.id);
            let b = b.clone();
            Box::pin(async move { Some(vec![b]) })
        });

        let find_value: FindValueFn = Arc::new(move |asked, _, k, _| {
            assert_eq!(k, 10);
            let value = if asked.id == a.id {
                stale.clone()
            } else {
                fresh.clone()
            };
            Box::pin(async move { Some(DhtValueResult::Found { value }) })
        });
        let priority = HashSet::new();

        let outcome = verify_value(
            "test verify",
            vec![a],
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            &priority,
            &cache,
            |value: &DhtValue| value.value == [1],
        )
        .await;

        assert_eq!(find_node_calls.load(Ordering::Relaxed), 1);
        assert!(outcome.served);
        assert_eq!(outcome.stale_values, 1);
        assert_eq!(outcome.stop_reason, "served");
    }

    #[tokio::test]
    async fn value_not_found_merges_nodes_and_continues() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let a = candidate(1, 44_001);
        let (b_node, b) = signed_dht_node(77, 44_002);
        assert_eq!(candidate_from_dht_node(&b_node).unwrap().id, b.id);
        let find_value: FindValueFn = Arc::new(move |asked, _, _, _| {
            let result = if asked.id == a.id {
                DhtValueResult::NotFound {
                    nodes: DhtNodes {
                        nodes: vec![b_node.clone()],
                    },
                }
            } else {
                DhtValueResult::Found {
                    value: dummy_value(1),
                }
            };
            Box::pin(async move { Some(result) })
        });
        let find_node: FindNodeFn = Arc::new(|_, _, _, _| Box::pin(async { None }));
        let priority = HashSet::new();

        let outcome = verify_value(
            "test verify",
            vec![a],
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            &priority,
            &cache,
            |value: &DhtValue| value.value == [1],
        )
        .await;

        assert!(outcome.served);
        assert_eq!(outcome.rounds, 2);
        assert_eq!(outcome.new_candidates, 1);
    }

    #[test]
    fn verification_candidates_deduplicate_by_node_id() {
        let key_id = [0u8; 32];
        let cache = DhtRoutingCache::new();
        let a = candidate(1, 45_001);
        let mut same_id = a.clone();
        same_id.address = "127.0.0.1:45999".parse().unwrap();
        let pending =
            verification_candidates(&key_id, &[], &[a], &[same_id], &cache, Instant::now());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn next_lookup_batch_skips_tried_node_ids() {
        let mut tried = HashSet::new();
        tried.insert([1u8; 32]);
        let batch = next_lookup_batch(
            &[candidate(1, 46_001), candidate(2, 46_002)],
            &[0; 32],
            &tried,
        );
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].id, [2; 32]);
    }

    #[test]
    fn verification_batch_queries_acknowledged_targets_first() {
        let key_id = [0u8; 32];
        let mut candidates: Vec<DhtCandidate> = (1..=9)
            .map(|id| candidate(id, 47_000 + u16::from(id)))
            .collect();
        let acknowledged = candidate(100, 47_100);
        candidates.push(acknowledged.clone());
        let priority = HashSet::from([acknowledged.id]);

        let batch = next_verification_batch(&candidates, &key_id, &HashSet::new(), &priority);

        assert_eq!(batch.len(), DHT_ALPHA);
        assert_eq!(batch[0].id, acknowledged.id);
        assert_eq!(
            batch
                .iter()
                .skip(1)
                .map(|candidate| candidate.id[0])
                .collect::<Vec<_>>(),
            (1..=7).collect::<Vec<_>>()
        );
    }
}
