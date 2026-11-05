//! Public-overlay DHT membership publishing.
//!
//! A public overlay has two independent DHT records:
//!
//! - `address` keyed by the node's ADNL id, published by the parent module;
//! - `overlay.<overlay_id>.nodes` keyed by the overlay id, whose value is an
//!   `overlay.nodes` list containing the node's signed `overlay.node` record.
//!
//! Upstream full nodes store the second record with
//! `dht.updateRule.overlayNodes`.  Publishing only the address makes a node
//! resolvable once another peer already knows its id, but it does not enter
//! the overlay's public DHT membership, so that peer has no route to discover
//! it in the first place.

use std::{collections::HashSet, net::SocketAddr, sync::Arc, time::Duration};

use crate::{
    adnl::{
        AdnlAddress, AdnlKeyPair, now_i32,
        tl::{
            AdnlIdShort, AdnlTlPublicKey as TlPublicKey, DhtKey, DhtKeyDescription, DhtUpdateRule,
            DhtValue, DhtValueResult,
        },
    },
    overlay::{
        OverlayId, SeedPeer,
        tl::{OverlayNode, OverlayNodeToSign, OverlayNodesBoxed},
        udp_session::{dht_key_id, valid_overlay_node},
    },
    tl::TlInt256 as Int256,
};

use super::{
    DhtAddressPublisher, DhtCandidate, PUBLISH_STORE_RETRY_DELAY, PUBLISH_STORE_RETRY_LONG_DELAY,
    PublishAttempt, VALUE_TTL_SECS, candidate_from_dht_node, known_candidates,
    publish_dht_value_attempt, xor_distance,
};

use futures::future::BoxFuture;

/// Per-query deadline for a membership `findValue` verification probe.
///
/// The publish timeout can be 30 seconds, but asking multiple enriched
/// candidates at that deadline would make verification outlast the fixed
/// republish cadence. A DHT node that has not answered a small `findValue`
/// within five seconds is not useful verification evidence.
const MEMBERSHIP_VERIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// Builds a membership publisher bound to this node's identity and overlay.
pub(super) fn membership_publisher(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: Vec<u8>,
    timeout: Duration,
) -> DhtAddressPublisher {
    let local_keypair = *local_keypair;
    Arc::new(move |nodes| {
        let overlay_name = overlay_name.clone();
        Box::pin(async move {
            publish_overlay_membership(
                local_addr,
                &local_keypair,
                overlay,
                overlay_name,
                nodes,
                timeout,
            )
            .await
        }) as BoxFuture<'static, usize>
    })
}

/// Publishes this node's `overlay.<id>.nodes` record through the bootstrap DHT
/// resolver set and verifies that `findValue` serves it back.
pub(super) async fn publish_overlay_membership(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: Vec<u8>,
    dht_nodes: Vec<SeedPeer>,
    timeout: Duration,
) -> usize {
    let first = publish_membership_attempt(
        local_addr,
        local_keypair,
        overlay,
        &overlay_name,
        &dht_nodes,
        timeout,
        0,
    )
    .await;
    let PublishAttempt {
        mut stored,
        mut candidates,
    } = first;
    for (index, delay) in [PUBLISH_STORE_RETRY_DELAY, PUBLISH_STORE_RETRY_LONG_DELAY]
        .into_iter()
        .enumerate()
    {
        if !stored.is_empty() {
            break;
        }
        let attempt = index + 1;
        log::debug!(
            "dht membership publish: attempt {attempt} stored nothing, retrying after {delay:?} with rotated resolvers"
        );
        tokio::time::sleep(delay).await;
        let retry = publish_membership_attempt(
            local_addr,
            local_keypair,
            overlay,
            &overlay_name,
            &dht_nodes,
            timeout,
            attempt,
        )
        .await;
        merge_attempt(&mut stored, &mut candidates, retry);
    }
    let served = verify_overlay_membership(
        local_addr,
        local_keypair,
        overlay,
        &overlay_name,
        &dht_nodes,
        &candidates,
        &stored,
        timeout,
    )
    .await;
    log::info!(
        "dht membership publish: overlay={overlay} stored on {} node(s), served_by_dht={served}",
        stored.len()
    );
    stored.len()
}

/// Merges one retry round into the accumulated placement evidence.
fn merge_attempt(
    stored: &mut Vec<DhtCandidate>,
    candidates: &mut Vec<DhtCandidate>,
    attempt: PublishAttempt,
) {
    for candidate in attempt.stored {
        if !stored
            .iter()
            .any(|existing| existing.address == candidate.address)
        {
            stored.push(candidate);
        }
    }
    for candidate in attempt.candidates {
        if !candidates
            .iter()
            .any(|existing| existing.address == candidate.address)
        {
            candidates.push(candidate);
        }
    }
}

async fn publish_membership_attempt(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
    dht_nodes: &[SeedPeer],
    timeout: Duration,
    attempt: usize,
) -> PublishAttempt {
    let value = membership_dht_value(local_keypair, overlay, overlay_name);
    let key_id = dht_key_id(overlay.as_bytes(), b"nodes").0;
    publish_dht_value_attempt(
        local_addr,
        *local_keypair,
        dht_nodes,
        key_id,
        value,
        timeout,
        attempt,
        "dht membership publish",
    )
    .await
}

/// Builds the signed `overlay.<id>.nodes` DHT value for this node.
pub(super) fn membership_dht_value(
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
) -> DhtValue {
    let now = now_i32();
    let unsigned_description = DhtKeyDescription {
        key: DhtKey {
            id: Int256(overlay.as_bytes()),
            name: b"nodes".to_vec(),
            idx: 0,
        },
        id: TlPublicKey::Overlay {
            name: overlay_name.to_vec(),
        },
        update_rule: DhtUpdateRule::OverlayNodes,
        signature: Vec::new(),
    };
    DhtValue {
        key: unsigned_description,
        value: tl_proto::serialize(OverlayNodesBoxed {
            nodes: vec![local_overlay_node(local_keypair, overlay)],
        }),
        ttl: now.saturating_add(VALUE_TTL_SECS),
        signature: Vec::new(),
    }
}

fn local_overlay_node(local_keypair: &AdnlKeyPair, overlay: OverlayId) -> OverlayNode {
    let version = now_i32();
    let adnl_id = AdnlAddress::from(&local_keypair.public_key).to_bytes();
    let to_sign = OverlayNodeToSign {
        id: AdnlIdShort {
            id: Int256(adnl_id),
        },
        overlay: Int256(overlay.as_bytes()),
        version,
    };
    OverlayNode {
        id: TlPublicKey::Ed25519 {
            key: Int256(local_keypair.public_key.to_bytes()),
        },
        overlay: Int256(overlay.as_bytes()),
        version,
        signature: local_keypair
            .sign_raw(&tl_proto::serialize(to_sign))
            .to_vec(),
    }
}

fn membership_value_matches(
    value: &DhtValue,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
) -> bool {
    if value.key.key
        != (DhtKey {
            id: Int256(overlay.as_bytes()),
            name: b"nodes".to_vec(),
            idx: 0,
        })
        || value.key.id
            != (TlPublicKey::Overlay {
                name: overlay_name.to_vec(),
            })
        || value.key.update_rule != DhtUpdateRule::OverlayNodes
        || !value.key.signature.is_empty()
        || !value.signature.is_empty()
        || OverlayId::from_name(overlay_name) != overlay
        || value.value.len() > 768
    {
        return false;
    }
    let Ok(nodes) = tl_proto::deserialize::<OverlayNodesBoxed>(&value.value) else {
        return false;
    };
    let now = now_i32();
    nodes.nodes.iter().any(|node| {
        node.id
            == (TlPublicKey::Ed25519 {
                key: Int256(local_keypair.public_key.to_bytes()),
            })
            && node.overlay.0 == overlay.as_bytes()
            && valid_overlay_node(node, overlay, now)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Verification needs the local identity, overlay key, resolver set, placement evidence, and deadline"
)]
async fn verify_overlay_membership(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
    dht_nodes: &[SeedPeer],
    publish_candidates: &[DhtCandidate],
    stored_targets: &[DhtCandidate],
    timeout: Duration,
) -> bool {
    let key_id = dht_key_id(overlay.as_bytes(), b"nodes");
    let query_timeout = timeout.min(MEMBERSHIP_VERIFY_TIMEOUT);
    let mut pending =
        membership_verification_candidates(dht_nodes, publish_candidates, stored_targets);
    if pending.is_empty() {
        log::debug!("dht membership verify: no nodes to ask");
        return false;
    }
    pending.sort_by_key(|candidate| xor_distance(&candidate.id, &key_id.0));
    let mut tried = HashSet::new();

    for round in 1..=3 {
        let mut batch = Vec::new();
        let mut rest = Vec::new();
        for candidate in pending.drain(..) {
            if !tried.insert(candidate.address) {
                continue;
            }
            if batch.len() < 4 {
                batch.push(candidate);
            } else {
                rest.push(candidate);
            }
        }
        if batch.is_empty() {
            break;
        }
        let results = futures::future::join_all(batch.into_iter().map(|candidate| {
            let key_id = key_id.clone();
            async move {
                let session = super::shared_session(
                    local_addr,
                    local_keypair,
                    candidate.public_key,
                    candidate.address,
                )
                .await
                .ok()?;
                let result = session
                    .dht_find_value(key_id, 1, query_timeout)
                    .await
                    .ok()?;
                Some((candidate, result))
            }
        }))
        .await;

        let mut answered = 0usize;
        for (candidate, result) in results.into_iter().flatten() {
            answered += 1;
            match result {
                DhtValueResult::Found { value } => {
                    let served =
                        membership_value_matches(&value, local_keypair, overlay, overlay_name);
                    if !served {
                        log_membership_mismatch(&value, local_keypair, overlay, overlay_name);
                    }
                    log::debug!(
                        "dht membership verify: served={served} (round {round}, node {})",
                        candidate.address
                    );
                    if served {
                        return true;
                    }
                }
                DhtValueResult::NotFound { nodes } => {
                    let now = now_i32();
                    for node in nodes.nodes {
                        if !node.is_valid(now) {
                            continue;
                        }
                        if let Some(closer) = candidate_from_dht_node(&node)
                            && !tried.contains(&closer.address)
                        {
                            rest.push(closer);
                        }
                    }
                }
            }
        }
        if answered == 0 {
            log::debug!("dht membership verify: walk round {round} got no answers");
            break;
        }
        pending = rest;
        if pending.is_empty() {
            break;
        }
    }
    log::debug!("dht membership verify: this node is not in the public overlay DHT yet");
    false
}

/// Chooses the nodes asked to prove that this membership record is served.
///
/// A successful `dht.store` only means at least one member of the target's
/// forwarding set accepted the value.  The target that answered can still
/// serve an older record, so verification starts from the complete enriched
/// placement set and treats acknowledged targets as additional evidence,
/// not as the only candidates.  When the store round produced no candidate
/// at all, fall back to the configured resolver seeds.
fn membership_verification_candidates(
    dht_nodes: &[SeedPeer],
    publish_candidates: &[DhtCandidate],
    stored_targets: &[DhtCandidate],
) -> Vec<DhtCandidate> {
    let mut pending = if publish_candidates.is_empty() {
        known_candidates(dht_nodes)
    } else {
        publish_candidates.to_vec()
    };
    for candidate in stored_targets {
        if !pending
            .iter()
            .any(|existing| existing.address == candidate.address)
        {
            pending.push(candidate.clone());
        }
    }
    pending
}

/// Logs why a `dht.findValue` answer does not prove this node's membership.
///
/// A failed membership verification is otherwise indistinguishable from a
/// rejected store or an evicted record, and the returned value is the only
/// evidence that distinguishes them.
fn log_membership_mismatch(
    value: &DhtValue,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
) {
    let expected_key = DhtKey {
        id: Int256(overlay.as_bytes()),
        name: b"nodes".to_vec(),
        idx: 0,
    };
    let local_id = TlPublicKey::Ed25519 {
        key: Int256(local_keypair.public_key.to_bytes()),
    };
    let now = now_i32();
    let parsed = tl_proto::deserialize::<OverlayNodesBoxed>(&value.value);
    let node_summary: Vec<String> = match &parsed {
        Ok(nodes) => nodes
            .nodes
            .iter()
            .map(|node| {
                format!(
                    "id_matches={} overlay_matches={} version={} valid={}",
                    node.id == local_id,
                    node.overlay.0 == overlay.as_bytes(),
                    node.version,
                    valid_overlay_node(node, overlay, now),
                )
            })
            .collect(),
        Err(error) => vec![format!("unparsable={error}")],
    };
    log::debug!(
        "dht membership verify: mismatch key_id_matches={} name={:?} idx={} key_id={:?} update_rule={:?} key_signature_len={} value_signature_len={} value_len={} name_hashes_back={} nodes={node_summary:?} value_hex={}",
        value.key.key == expected_key,
        String::from_utf8_lossy(&value.key.key.name),
        value.key.key.idx,
        value.key.id,
        value.key.update_rule,
        value.key.signature.len(),
        value.signature.len(),
        value.value.len(),
        OverlayId::from_name(overlay_name) == overlay,
        hex::encode(&value.value),
    );
}

#[cfg(test)]
mod tests {
    use crate::{
        adnl::{AdnlKeyPair, tl::DhtUpdateRule},
        overlay::{OverlayId, SeedPeer, tl::OverlayNodesBoxed},
    };

    use super::{
        DhtCandidate, membership_dht_value, membership_value_matches,
        membership_verification_candidates,
    };

    #[test]
    fn membership_value_is_signed_for_the_public_overlay_key() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let name = b"membership-test-overlay";
        let overlay = OverlayId::from_name(name);
        let value = membership_dht_value(&keypair, overlay, name);

        assert_eq!(value.key.key.id.0, overlay.as_bytes());
        assert_eq!(value.key.key.name, b"nodes");
        assert_eq!(value.key.key.idx, 0);
        assert!(matches!(
            value.key.id,
            crate::adnl::tl::AdnlTlPublicKey::Overlay { .. }
        ));
        assert_eq!(value.key.update_rule, DhtUpdateRule::OverlayNodes);
        assert_eq!(value.key.signature, Vec::<u8>::new());
        assert_eq!(value.signature, Vec::<u8>::new());
        assert!(value.value.len() <= 768);

        let nodes: OverlayNodesBoxed = tl_proto::deserialize(&value.value).unwrap();
        assert_eq!(nodes.nodes.len(), 1);
        assert!(membership_value_matches(&value, &keypair, overlay, name));
    }

    #[test]
    fn membership_value_rejects_another_overlay_name() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let name = b"membership-test-overlay";
        let overlay = OverlayId::from_name(name);
        let value = membership_dht_value(&keypair, overlay, name);

        assert!(!membership_value_matches(
            &value,
            &keypair,
            OverlayId::from_name(b"another-overlay"),
            name,
        ));
        assert!(!membership_value_matches(
            &value,
            &keypair,
            overlay,
            b"another-overlay",
        ));
    }

    #[test]
    fn verification_keeps_all_enriched_candidates_and_store_targets() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let enriched = DhtCandidate {
            id: [1; 32],
            public_key: keypair.public_key,
            address: "127.0.0.1:30303".parse().unwrap(),
        };
        let acknowledged = DhtCandidate {
            id: [2; 32],
            public_key: keypair.public_key,
            address: "127.0.0.1:30304".parse().unwrap(),
        };
        let seeds = vec![SeedPeer::from_public_key(
            keypair.public_key.to_bytes(),
            "127.0.0.1:30305",
        )];

        let candidates = membership_verification_candidates(
            &seeds,
            std::slice::from_ref(&enriched),
            std::slice::from_ref(&acknowledged),
        );

        assert_eq!(candidates.len(), 2);
        assert!(
            candidates
                .iter()
                .any(|node| node.address == enriched.address)
        );
        assert!(
            candidates
                .iter()
                .any(|node| node.address == acknowledged.address)
        );
    }
}
