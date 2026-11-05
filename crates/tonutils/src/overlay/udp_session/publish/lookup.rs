//! Shared bounded DHT placement and verification primitives.
//!
//! The address and public-membership publishers both need the same
//! iterative `findNode` lookup, the same `k`-closest store set, and the same
//! `findValue` verification walk. Keeping those mechanics here avoids the two
//! publishers drifting apart and makes the walk testable without a live DHT.

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
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

/// The same `k` in the signed wire field's width.
const DHT_K_I32: i32 = 10;

/// How many store targets one publish round uses.
pub(super) const STORE_TARGETS: usize = 10;

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

/// A DHT node a publish round can query or store a value on.
#[derive(Clone, Debug)]
pub(super) struct DhtCandidate {
    /// The node's position in the DHT keyspace: its ADNL short id.
    pub(super) id: [u8; 32],
    pub(super) public_key: AdnlPublicKey,
    pub(super) address: SocketAddr,
}

/// Outcome of one DHT store round.
///
/// `acknowledged` contains only nodes that answered `dht.stored`. That answer
/// confirms the store request was delivered; it does not prove the node kept
/// the value or that `findValue` can serve it. `candidates` is the enriched,
/// distance-sorted placement set used for those stores.
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
    pub(super) candidates: Vec<DhtCandidate>,
    pub(super) rounds: usize,
    pub(super) asked: usize,
    pub(super) answered: usize,
    pub(super) new_candidates: usize,
    pub(super) stop_reason: &'static str,
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

/// Returns the deadline for one bounded DHT attempt.
pub(super) fn attempt_deadline(timeout: Duration) -> Instant {
    Instant::now() + timeout.min(DHT_ATTEMPT_BUDGET)
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

/// The full verification seed set: bootstrap resolvers, enriched lookup
/// candidates, and nodes that acknowledged `dht.store`, deduplicated by DHT
/// node id and sorted by distance to the key.
pub(super) fn verification_candidates(
    key_id: &[u8; 32],
    dht_nodes: &[SeedPeer],
    publish_candidates: &[DhtCandidate],
    acknowledged_targets: &[DhtCandidate],
) -> Vec<DhtCandidate> {
    let mut pending = known_candidates(dht_nodes);
    for candidate in publish_candidates.iter().chain(acknowledged_targets) {
        let _ = insert_candidate(&mut pending, candidate.clone());
    }
    sort_candidates(&mut pending, key_id);
    pending
}

/// Selects the closest store targets. `dht.store` is sent only to these.
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
/// distance-sorted candidate set.
///
/// Each wave asks the closest untried candidates. The walk stops as soon as
/// no untried candidate is closer than the current tenth result, when a wave
/// or query budget is exhausted, or when the attempt deadline passes.
pub(super) async fn lookup_closest(
    operation: &'static str,
    attempt: usize,
    mut candidates: Vec<DhtCandidate>,
    key_id: [u8; 32],
    deadline: Instant,
    query: FindNodeFn,
) -> LookupOutcome {
    let mut tried = HashSet::new();
    let mut rounds = 0usize;
    let mut asked = 0usize;
    let mut answered = 0usize;
    let mut new_candidates = 0usize;
    let mut stop_reason = "candidate_exhausted";

    for wave in 1..=DHT_LOOKUP_WAVES {
        sort_candidates(&mut candidates, &key_id);
        let batch = next_lookup_batch(&candidates, &key_id, &tried);
        if batch.is_empty() {
            stop_reason = if candidates.len() >= DHT_K {
                "converged"
            } else {
                "candidate_exhausted"
            };
            break;
        }
        if candidates.len() >= DHT_K {
            let kth_distance = xor_distance(&candidates[DHT_K - 1].id, &key_id);
            let nearest_untried = xor_distance(&batch[0].id, &key_id);
            if nearest_untried >= kth_distance {
                stop_reason = "converged";
                break;
            }
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
        asked += batch_len;

        let results = join_all(batch.into_iter().map(|candidate| {
            let query = Arc::clone(&query);
            async move {
                let result = query(candidate.clone(), key_id, DHT_K_I32, timeout).await;
                (candidate, result)
            }
        }))
        .await;

        let mut wave_answered = 0usize;
        let mut wave_new = 0usize;
        for (candidate, result) in results {
            let Some(nodes) = result else {
                continue;
            };
            wave_answered += 1;
            for node in nodes {
                if insert_candidate(&mut candidates, node) {
                    wave_new += 1;
                }
            }
            log::debug!(
                "{operation}: findNode via {} answered on wave {wave}",
                candidate.address
            );
        }
        answered += wave_answered;
        new_candidates += wave_new;
        log::debug!(
            "{operation}: lookup attempt={attempt} wave={wave} asked={batch_len} answered={wave_answered} new_candidates={wave_new} candidates={}",
            candidates.len()
        );
    }

    sort_candidates(&mut candidates, &key_id);
    log::debug!(
        "{operation}: lookup attempt={attempt} rounds={rounds} asked={asked} answered={answered} new_candidates={new_candidates} candidates={} stop={stop_reason}",
        candidates.len()
    );
    LookupOutcome {
        candidates,
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
    reason = "One verification wave must keep answer handling, stale follow-up, and counters together"
)]
pub(super) async fn verify_value<F>(
    operation: &'static str,
    mut pending: Vec<DhtCandidate>,
    key_id: [u8; 32],
    deadline: Instant,
    find_value: FindValueFn,
    find_node: FindNodeFn,
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
    let mut stop_reason = "candidate_exhausted";

    for wave in 1..=DHT_LOOKUP_WAVES {
        sort_candidates(&mut pending, &key_id);
        let batch = next_lookup_batch(&pending, &key_id, &tried);
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
        asked += batch_len;

        let results = join_all(batch.into_iter().map(|candidate| {
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
        for (candidate, result) in results {
            let Some(result) = result else {
                continue;
            };
            wave_answered += 1;
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
                        if let Some(closer) = candidate_from_dht_node(&node)
                            && insert_candidate(&mut pending, closer)
                        {
                            wave_new += 1;
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
                    if insert_candidate(&mut pending, node) {
                        wave_new += 1;
                    }
                }
            }
        }
        new_candidates += wave_new;
        log::debug!(
            "{operation}: wave={wave} asked={batch_len} answered={wave_answered} new_candidates={wave_new} stale_values={stale_values} pending={}",
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
        DHT_K, DhtCandidate, FindNodeFn, FindValueFn, STORE_TARGETS, candidate_from_dht_node,
        lookup_closest, next_lookup_batch, short_id, store_targets, verification_candidates,
        verify_value,
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
            query,
        )
        .await;

        assert!(outcome.candidates.len() >= DHT_K);
        let ids: Vec<u8> = outcome
            .candidates
            .iter()
            .take(DHT_K)
            .map(|candidate| candidate.id[0])
            .collect();
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        assert_eq!(outcome.stop_reason, "converged");
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

    #[tokio::test]
    async fn acknowledged_store_is_not_success_without_a_served_value() {
        let key_id = [0u8; 32];
        let a = candidate(1, 42_001);
        let pending = verification_candidates(
            &key_id,
            &[],
            std::slice::from_ref(&a),
            std::slice::from_ref(&a),
        );
        let find_value_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&find_value_calls);
        let find_value: FindValueFn = Arc::new(move |_, _, _, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Some(empty_not_found()) })
        });
        let find_node: FindNodeFn = Arc::new(|_, _, _, _| Box::pin(async { None }));

        let outcome = verify_value(
            "test verify",
            pending,
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            |_| false,
        )
        .await;

        assert!(!outcome.served);
        assert_eq!(find_value_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn stale_value_found_triggers_find_node_and_then_serves() {
        let key_id = [0u8; 32];
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

        let outcome = verify_value(
            "test verify",
            vec![a],
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            |value| value.value == [1],
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

        let outcome = verify_value(
            "test verify",
            vec![a],
            key_id,
            Instant::now() + Duration::from_secs(5),
            find_value,
            find_node,
            |value| value.value == [1],
        )
        .await;

        assert!(outcome.served);
        assert_eq!(outcome.rounds, 2);
        assert_eq!(outcome.new_candidates, 1);
    }

    #[test]
    fn verification_candidates_deduplicate_by_node_id() {
        let key_id = [0u8; 32];
        let a = candidate(1, 45_001);
        let mut same_id = a.clone();
        same_id.address = "127.0.0.1:45999".parse().unwrap();
        let pending = verification_candidates(&key_id, &[], &[a], &[same_id]);
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
}
