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

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    adnl::{
        AdnlAddress, AdnlKeyPair, now_i32,
        tl::{
            AdnlIdShort, AdnlTlPublicKey as TlPublicKey, DhtKey, DhtKeyDescription, DhtUpdateRule,
            DhtValue,
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
    DhtAddressPublisher, VALUE_TTL_SECS,
    lookup::{
        DhtCandidate, PublishAttempt, attempt_deadline, find_node_requester, find_value_requester,
        merge_attempt, verification_candidates, verify_value,
    },
    publish_dht_value_attempt,
};

use futures::future::BoxFuture;

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
    let mut acknowledged = Vec::new();
    let mut candidates = Vec::new();
    let mut served = false;
    for (attempt, delay) in [
        None,
        Some(super::PUBLISH_STORE_RETRY_DELAY),
        Some(super::PUBLISH_STORE_RETRY_LONG_DELAY),
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let deadline = attempt_deadline(timeout);
        let publish = publish_membership_attempt(
            local_addr,
            local_keypair,
            overlay,
            &overlay_name,
            &dht_nodes,
            deadline,
            attempt,
        )
        .await;
        merge_attempt(&mut acknowledged, &mut candidates, publish);
        served = verify_overlay_membership(
            local_addr,
            local_keypair,
            overlay,
            &overlay_name,
            &dht_nodes,
            &candidates,
            &acknowledged,
            deadline,
        )
        .await;
        if served {
            break;
        }
        log::debug!(
            "dht membership publish: attempt {attempt} acknowledged={} but served_by_dht=false",
            acknowledged.len()
        );
    }
    log::info!(
        "dht membership publish: overlay={overlay} acknowledged={} served_by_dht={served}",
        acknowledged.len()
    );
    acknowledged.len()
}

async fn publish_membership_attempt(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    overlay: OverlayId,
    overlay_name: &[u8],
    dht_nodes: &[SeedPeer],
    deadline: Instant,
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
        deadline,
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
    acknowledged_targets: &[DhtCandidate],
    deadline: Instant,
) -> bool {
    let key_id = dht_key_id(overlay.as_bytes(), b"nodes").0;
    let pending =
        verification_candidates(&key_id, dht_nodes, publish_candidates, acknowledged_targets);
    let priority: HashSet<_> = acknowledged_targets
        .iter()
        .map(|target| target.id)
        .collect();
    if pending.is_empty() {
        log::debug!("dht membership verify: no nodes to ask");
        return false;
    }
    let keypair = *local_keypair;
    let overlay_name = overlay_name.to_vec();
    let matches = move |value: &DhtValue| {
        let served = membership_value_matches(value, &keypair, overlay, &overlay_name);
        if !served {
            log_membership_mismatch(value, &keypair, overlay, &overlay_name);
        }
        served
    };
    let outcome = verify_value(
        "dht membership verify",
        pending,
        key_id,
        deadline,
        find_value_requester(local_addr, &keypair),
        find_node_requester(local_addr, &keypair),
        &priority,
        matches,
    )
    .await;
    log::info!(
        "dht membership verify: overlay={overlay} seeds={} lookup_candidates={} acknowledged={} served_by_dht={} rounds={} asked={} answered={} new_candidates={} stale_values={} stop={}",
        dht_nodes.len(),
        publish_candidates.len(),
        acknowledged_targets.len(),
        outcome.served,
        outcome.rounds,
        outcome.asked,
        outcome.answered,
        outcome.new_candidates,
        outcome.stale_values,
        outcome.stop_reason
    );
    outcome.served
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
        DhtCandidate, membership_dht_value, membership_value_matches, verification_candidates,
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

        let candidates = verification_candidates(
            &[9; 32],
            &seeds,
            std::slice::from_ref(&enriched),
            std::slice::from_ref(&acknowledged),
        );

        assert_eq!(candidates.len(), 3);
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
