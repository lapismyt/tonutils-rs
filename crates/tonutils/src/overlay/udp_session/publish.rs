#![expect(
    clippy::cast_possible_wrap,
    reason = "ADNL and overlay wire fields and bounded counters intentionally use protocol widths"
)]
//! DHT publication for the native UDP path.
//!
//! Overlay peers resolve a node's ADNL address through the DHT
//! `address` value keyed by the node's ADNL id
//! (`DhtKey { id: short_id, name: "address", idx: 0 }`) - the
//! same lookup [`resolve_address`](super::lookup) performs for
//! every overlay candidate.  Upstream publishes that value
//! whenever the address list changes (`publish_address_list` in
//! `adnl/adnl-local-id.cpp`), and pytoniq's example client stores
//! it with `dht.store`.  A node that never publishes can only be
//! reached by peers it contacted itself: every other peer that
//! learns its signed record from an `overlay.getRandomPeers`
//! answer has no way to resolve the address and cannot ping it,
//! which is what stops transitive peer discovery from
//! compounding.
//!
//! Publishing is a thin-client operation: the value is signed by
//! the ADNL key pair, the ten DHT nodes closest to the key are located
//! with a bounded iterative `dht.findNode` walk, and the value is stored
//! with `dht.store` on those candidates, mirroring upstream's `k = 10`
//! replication factor. Upstream DHT nodes accept a store only when they are
//! responsible for the key and drop it otherwise, so storing on the closest
//! known candidates is what makes the value retrievable. A `dht.stored`
//! answer is only a store-request acknowledgement; publication succeeds only
//! after `dht.findValue` serves the local signed record.
//!
//! A public overlay full node also publishes its membership record through
//! [`membership`], because the address record alone does not make it
//! discoverable as an overlay member.

use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    adnl::{
        AdnlKeyPair, AdnlUdpTransport, now_i32,
        tl::{
            AddressListBoxed, AdnlNetAddress as Address, AdnlTlPublicKey as TlPublicKey, DhtKey,
            DhtKeyDescription, DhtMessage, DhtUpdateRule, DhtValue, DhtValueBare, Int128,
        },
        udp::local_reinit_date,
    },
    overlay::udp_session::{dht_key_id, shared_session, stun::discover_mapped_address},
    overlay::{OverlayId, SeedPeer},
    tl::TlInt256 as Int256,
};

use futures::future::BoxFuture;

mod lookup;
mod membership;

use lookup::{
    DHT_STORE_TIMEOUT, DhtCandidate, PublishAttempt, attempt_deadline, find_node_requester,
    find_value_requester, known_candidates, lookup_closest, merge_attempt, short_id, store_targets,
    verification_candidates, verify_value,
};

/// Lifetime of a published `address` value.
///
/// Upstream sets `ttl = now + 3600` and the DHT rejects anything
/// longer than an hour plus a minute, so one hour is the maximum
/// useful lifetime.  The publishing task re-publishes well inside
/// it.
const VALUE_TTL_SECS: i32 = 3600;

/// Interval between publish rounds for the DHT `address` value.
///
/// A published value lives for `VALUE_TTL_SECS`, so re-publishing
/// every ten minutes keeps it fresh with a wide margin even when a
/// round fails.
pub const PUBLISH_INTERVAL: Duration = Duration::from_mins(10);

/// Interval between publish-task rounds.
///
/// The public-overlay membership value is capped at 768 bytes and the DHT
/// update rule randomly evicts existing nodes when a new node no longer fits.
/// Upstream republishes that value every 60-100 seconds. A fresh node loses
/// the admission race against the rest of the overlay at that rate, so this
/// crate publishes twice as often while keeping each round bounded. The task
/// wakes on this interval and publishes the membership record every round;
/// the address record keeps the ten-minute [`PUBLISH_INTERVAL`] cadence
/// through the combined publisher.
pub(crate) const MEMBERSHIP_PUBLISH_INTERVAL: Duration = Duration::from_secs(30);

/// How many publish-task rounds share one `address` publication.
const ADDRESS_PUBLISH_EVERY: u64 =
    PUBLISH_INTERVAL.as_secs() / MEMBERSHIP_PUBLISH_INTERVAL.as_secs();

/// Delay before one bounded retry of a round that could not serve the value.
///
/// A responsible node that is momentarily unreachable answers nothing at
/// all, which the round cannot tell apart from a rejection, so retries stop
/// only after `findValue` serves this node's own record.
const PUBLISH_STORE_RETRY_DELAY: Duration = Duration::from_secs(5);

/// The second retry delay, longer than the first: the recorded
/// round-wide failures (TODO item 447) stayed silent across the whole
/// five-second window, so a second, wider beat gives a shaped or
/// reloading path time to recover before the next interval.
const PUBLISH_STORE_RETRY_LONG_DELAY: Duration = Duration::from_secs(15);

/// Upper bound of the random first-round stagger.
///
/// Parallel scanners behind one NAT address (the live delivery gate)
/// start their STUN probes and store rounds within the same second,
/// which correlates the candidate silence the retries exist for; a
/// per-instance jitter spreads the first round over this window.
/// Later rounds stay on the fixed [`PUBLISH_INTERVAL`] cadence.
pub const PUBLISH_FIRST_ROUND_STAGGER_MAX: Duration = Duration::from_secs(20);

/// Builds the signed DHT `address` value for this node.
///
/// The value carries the externally reachable UDP address in an
/// `adnl.addressList`, signed by the ADNL key pair the same way
/// upstream signs it in `publish_address_list`: both the key
/// description and the value itself are signed over their boxed
/// serialization with the signature field still empty.
pub fn address_dht_value(local_keypair: &AdnlKeyPair, external: SocketAddr) -> DhtValue {
    let version = now_i32();
    let address_list = AddressListBoxed {
        addrs: vec![address_for(external)],
        version,
        reinit_date: local_reinit_date(),
        priority: 0,
        expire_at: 0,
    };
    let unsigned_description = DhtKeyDescription {
        key: DhtKey {
            id: Int256(short_id(&local_keypair.public_key)),
            name: b"address".to_vec(),
            idx: 0,
        },
        id: TlPublicKey::Ed25519 {
            key: Int256(local_keypair.public_key.to_bytes()),
        },
        update_rule: DhtUpdateRule::Signature,
        signature: Vec::new(),
    };
    let key_description = DhtKeyDescription {
        signature: local_keypair
            .sign_raw(&unsigned_description.unsigned_bytes())
            .to_vec(),
        ..unsigned_description
    };
    let unsigned_value = DhtValue {
        key: key_description,
        value: tl_proto::serialize(address_list),
        ttl: version.saturating_add(VALUE_TTL_SECS),
        signature: Vec::new(),
    };
    DhtValue {
        signature: local_keypair
            .sign_raw(&unsigned_value.unsigned_bytes())
            .to_vec(),
        ..unsigned_value
    }
}

/// The `adnl.address` form of an externally reachable socket
/// address.
fn address_for(external: SocketAddr) -> Address {
    match external.ip() {
        IpAddr::V4(ip) => Address::Udp {
            ip: u32::from(ip) as i32,
            port: external.port().into(),
        },
        IpAddr::V6(ip) => {
            let octets = ip.octets();
            Address::Udp6 {
                ip: Int128(
                    u32::from_be_bytes(octets[0..4].try_into().expect("4 bytes")) as i32,
                    u32::from_be_bytes(octets[4..8].try_into().expect("4 bytes")) as i32,
                    u32::from_be_bytes(octets[8..12].try_into().expect("4 bytes")) as i32,
                    u32::from_be_bytes(octets[12..16].try_into().expect("4 bytes")) as i32,
                ),
                port: external.port().into(),
            }
        }
    }
}

/// Whether an IP address is globally routable and therefore worth
/// publishing in the DHT.
///
/// Behind NAT the route probe would otherwise return a private
/// address that every DHT node fails to reach, poisoning the
/// `address` value for its whole lifetime.
fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && !ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unicast_link_local()
                && !ip.is_unique_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
        }
    }
}

/// Whether [`is_globally_routable`] accepts the IP of `address`.
pub(super) fn is_publishable_ip(address: SocketAddr) -> bool {
    is_globally_routable(address.ip())
}

/// Detects the externally reachable UDP address of this host.
///
/// A UDP socket connected to a remote address makes the operating
/// system pick the source address it routes through without
/// sending a datagram, so the probe is side-effect free.  The
/// probe's address is combined with `bound_port`, the port the
/// scanner's shared socket is bound to.  Only globally routable
/// addresses are accepted, see [`is_globally_routable`].
pub async fn detect_external_udp_address(
    probes: &[SocketAddr],
    bound_port: u16,
) -> Option<SocketAddr> {
    for probe in probes {
        let Ok(socket) = tokio::net::UdpSocket::bind("0.0.0.0:0").await else {
            continue;
        };
        if socket.connect(probe).await.is_err() {
            continue;
        }
        if let Ok(local) = socket.local_addr()
            && is_globally_routable(local.ip())
        {
            return Some(SocketAddr::new(local.ip(), bound_port));
        }
    }
    None
}

/// One publish round with a fresh bounded attempt budget.
#[expect(
    clippy::large_types_passed_by_value,
    reason = "The signature intentionally mirrors protocol flags and values moved across async task boundaries"
)]
async fn publish_dht_address_value_attempt(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    external: SocketAddr,
    dht_nodes: &[SeedPeer],
    deadline: Instant,
    attempt: usize,
) -> PublishAttempt {
    let value = address_dht_value(&local_keypair, external);
    let key_id = dht_key_id(short_id(&local_keypair.public_key), b"address").0;
    publish_dht_value_attempt(
        local_addr,
        local_keypair,
        dht_nodes,
        key_id,
        value,
        deadline,
        attempt,
        "dht address publish",
    )
    .await
}

/// Stores one signed DHT value on the ten nodes closest to `key_id`.
///
/// The address publisher and the overlay-membership publisher share this
/// placement logic; only the value and key differ.
#[expect(
    clippy::large_types_passed_by_value,
    clippy::too_many_arguments,
    reason = "The signature intentionally mirrors protocol flags and values moved across async task boundaries"
)]
async fn publish_dht_value_attempt(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    dht_nodes: &[SeedPeer],
    key_id: [u8; 32],
    value: DhtValue,
    deadline: Instant,
    attempt: usize,
    operation: &'static str,
) -> PublishAttempt {
    let store_query = tl_proto::serialize(DhtMessage::Store {
        value: DhtValueBare::from(&value),
    });
    // A DHT node that rejects the stored value answers nothing at
    // all - the client only sees a timeout - so the exact payload is
    // the primary evidence when a store round fails.
    log::debug!(
        "{operation}: dht.store payload ({} bytes): {}",
        store_query.len(),
        hex::encode(&store_query)
    );
    log::debug!(
        "{operation}: attempt={attempt} seeds={} starting k={} lookup",
        dht_nodes.len(),
        lookup::DHT_K
    );
    let lookup = lookup_closest(
        operation,
        attempt,
        known_candidates(dht_nodes),
        key_id,
        deadline,
        find_node_requester(local_addr, &local_keypair),
    )
    .await;
    let targets = store_targets(&lookup.candidates, &key_id);
    let remaining = deadline.saturating_duration_since(Instant::now());
    let store_timeout = DHT_STORE_TIMEOUT.min(remaining);
    let mut acknowledged = Vec::new();
    if !targets.is_empty() && !remaining.is_zero() {
        // The stores are independent of each other, so run them
        // concurrently instead of waiting for each round trip in turn.
        let stores = targets.iter().cloned().map(|candidate| {
            let value = value.clone();
            async move {
                let session = match shared_session(
                    local_addr,
                    &local_keypair,
                    candidate.public_key,
                    candidate.address,
                )
                .await
                {
                    Ok(session) => session,
                    Err(error) => {
                        log::debug!(
                            "{operation}: session to {} failed: {error}",
                            candidate.address
                        );
                        return None;
                    }
                };
                if let Err(error) = session.dht_store(value, store_timeout).await {
                    log::debug!(
                        "{operation}: store on {} failed: {error}",
                        candidate.address
                    );
                    return None;
                }
                Some(candidate)
            }
        });
        acknowledged = futures::future::join_all(stores)
            .await
            .into_iter()
            .flatten()
            .collect();
    }
    log::debug!(
        "{operation}: attempt={attempt} lookup_rounds={} asked={} answered={} new_candidates={} candidates={} stop={} store_targets={} acknowledged={}",
        lookup.rounds,
        lookup.asked,
        lookup.answered,
        lookup.new_candidates,
        lookup.candidates.len(),
        lookup.stop_reason,
        targets.len(),
        acknowledged.len()
    );
    PublishAttempt {
        acknowledged,
        candidates: lookup.candidates,
    }
}

/// Whether a retrieved value is the current `address` value of this
/// node: owned by this key pair, signed, and carrying the
/// externally reachable address the round published.
fn value_matches(value: &DhtValue, local_keypair: &AdnlKeyPair, external: SocketAddr) -> bool {
    let TlPublicKey::Ed25519 { key } = &value.key.id else {
        return false;
    };
    if key.0 != local_keypair.public_key.to_bytes() || !value.verify_signature() {
        log::debug!("dht address verify: retrieved value does not belong to this node");
        return false;
    }
    let Ok(address_list) = tl_proto::deserialize::<AddressListBoxed>(&value.value) else {
        log::debug!("dht address verify: retrieved value is not an address list");
        return false;
    };
    address_list
        .addrs
        .iter()
        .any(|address| match (address, external.ip()) {
            (Address::Udp { ip, port }, IpAddr::V4(expected)) => {
                *ip == u32::from(expected) as i32 && *port == i32::from(external.port())
            }
            _ => false,
        })
}

#[expect(
    clippy::large_types_passed_by_value,
    reason = "The verifier keeps the local identity and deadline together for the bounded attempt"
)]
async fn verify_dht_address_value_until(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    external: SocketAddr,
    dht_nodes: &[SeedPeer],
    publish_candidates: &[DhtCandidate],
    acknowledged_targets: &[DhtCandidate],
    deadline: Instant,
) -> bool {
    let key_id = dht_key_id(short_id(&local_keypair.public_key), b"address").0;
    let pending =
        verification_candidates(&key_id, dht_nodes, publish_candidates, acknowledged_targets);
    if pending.is_empty() {
        log::debug!("dht address verify: no nodes to ask");
        return false;
    }
    let outcome = verify_value(
        "dht address verify",
        pending,
        key_id,
        deadline,
        find_value_requester(local_addr, &local_keypair),
        find_node_requester(local_addr, &local_keypair),
        move |value| value_matches(value, &local_keypair, external),
    )
    .await;
    log::info!(
        "dht address verify: seeds={} lookup_candidates={} acknowledged={} served_by_dht={} rounds={} asked={} answered={} new_candidates={} stale_values={} stop={}",
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

/// Publisher of this node's DHT records for the native UDP path.
///
/// One call runs a full publish round: it resolves the externally
/// reachable address, stores the signed `address` value and, for a public
/// overlay, the `overlay.<id>.nodes` membership value on the DHT nodes
/// closest to their keys, verifies the values are served, and returns how
/// many nodes acknowledged `dht.stored` for the address record.
pub type DhtAddressPublisher =
    Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, usize> + Send + Sync>;

/// Builds the publisher the native UDP path installs on the
/// overlay node builder.
///
/// The publisher resolves the external address lazily on every
/// round: the address configured through
/// [`crate::overlay::OverlayNodeBuilder::external_address`], then the
/// `TON_MEMPOOL_EXTERNAL_ADDRESS` environment variable, then a
/// route probe against the known DHT nodes.  A lazy resolution
/// keeps the value correct when the host's address changes
/// between rounds. A round whose value is not served is retried up to
/// twice with five- and fifteen-second delays, because a silent responsible
/// node is indistinguishable from a transient failure. Retries stop as soon
/// as `findValue` serves this node's own record.
pub fn address_publisher(
    local_addr: SocketAddr,
    local_keypair: AdnlKeyPair,
    configured_external: Option<SocketAddr>,
    timeout: Duration,
) -> DhtAddressPublisher {
    Arc::new(move |nodes: Vec<SeedPeer>| {
        let local_addr = local_addr;
        let local_keypair = local_keypair;
        let configured_external = configured_external;
        Box::pin(async move {
            let probes: Vec<SocketAddr> = nodes
                .iter()
                .filter_map(|node| node.address.parse().ok())
                .collect();
            let Some(external) = resolve_external_udp_address(
                local_addr,
                &local_keypair,
                configured_external,
                &probes,
            )
            .await
            else {
                log::debug!("dht address publish: no externally reachable address for this node");
                return 0;
            };
            let mut acknowledged = Vec::new();
            let mut candidates = Vec::new();
            let mut served = false;
            for (attempt, delay) in [
                None,
                Some(PUBLISH_STORE_RETRY_DELAY),
                Some(PUBLISH_STORE_RETRY_LONG_DELAY),
            ]
            .into_iter()
            .enumerate()
            {
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                let deadline = attempt_deadline(timeout);
                let publish = publish_dht_address_value_attempt(
                    local_addr,
                    local_keypair,
                    external,
                    &nodes,
                    deadline,
                    attempt,
                )
                .await;
                merge_attempt(&mut acknowledged, &mut candidates, publish);
                served = verify_dht_address_value_until(
                    local_addr,
                    local_keypair,
                    external,
                    &nodes,
                    &candidates,
                    &acknowledged,
                    deadline,
                )
                .await;
                if served {
                    break;
                }
                log::debug!(
                    "dht address publish: attempt {attempt} acknowledged={} but served_by_dht=false",
                    acknowledged.len()
                );
            }
            log::info!(
                "dht address publish: acknowledged={} served_by_dht={served}",
                acknowledged.len()
            );
            acknowledged.len()
        })
    })
}

/// Builds the native UDP publisher for both DHT records a public overlay node
/// needs: its `address` value and, when the overlay name is known, its
/// `overlay.<id>.nodes` membership value.
///
/// The membership half is skipped for an arbitrary overlay ID because the ID is
/// a hash and the DHT key description must carry the original `pub.overlay`
/// name that hashes back to it.
pub(crate) fn overlay_publisher(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    configured_external: Option<SocketAddr>,
    timeout: Duration,
    overlay: OverlayId,
    overlay_name: Option<Vec<u8>>,
) -> DhtAddressPublisher {
    let address = address_publisher(local_addr, *local_keypair, configured_external, timeout);
    let membership = overlay_name.map(|overlay_name| {
        membership::membership_publisher(local_addr, local_keypair, overlay, overlay_name, timeout)
    });
    combine_publishers(address, membership)
}

/// Runs address publication and, when configured, overlay-membership
/// publication against the same resolver set.
fn combine_publishers(
    address: DhtAddressPublisher,
    membership: Option<DhtAddressPublisher>,
) -> DhtAddressPublisher {
    let Some(membership) = membership else {
        return address;
    };
    let rounds = Arc::new(AtomicU64::new(0));
    Arc::new(move |nodes| {
        let address = Arc::clone(&address);
        let membership = Arc::clone(&membership);
        let rounds = Arc::clone(&rounds);
        Box::pin(async move {
            let round = rounds.fetch_add(1, Ordering::Relaxed) + 1;
            let publish_address = round == 1 || round.is_multiple_of(ADDRESS_PUBLISH_EVERY);
            log::debug!(
                "dht publish: round={round} records={}",
                if publish_address {
                    "address,overlay_nodes"
                } else {
                    "overlay_nodes"
                }
            );
            if publish_address {
                let (acknowledged, _) = futures::join!(address(nodes.clone()), membership(nodes));
                acknowledged
            } else {
                membership(nodes).await;
                0
            }
        })
    })
}

/// Resolves the externally reachable UDP address for a publish
/// round: the configured address, then the
/// `TON_MEMPOOL_EXTERNAL_ADDRESS` environment variable, then a
/// route probe against the known DHT nodes, then STUN.
///
/// The route probe cannot see a NAT boundary, so on a NAT'd host it
/// reports a private address and the round falls through to
/// [`super::stun::discover_mapped_address`], which asks the transport
/// socket itself where it is reachable.
async fn resolve_external_udp_address(
    local_addr: SocketAddr,
    local_keypair: &AdnlKeyPair,
    configured: Option<SocketAddr>,
    probes: &[SocketAddr],
) -> Option<SocketAddr> {
    if let Some(external) = configured {
        return Some(external);
    }
    if let Ok(value) = std::env::var("TON_MEMPOOL_EXTERNAL_ADDRESS")
        && let Ok(external) = value.parse()
    {
        return Some(external);
    }
    let transport = AdnlUdpTransport::for_node(local_addr, *local_keypair)
        .await
        .ok()?;
    let bound_port = transport.local_addr().ok()?.port();
    if let Some(external) = detect_external_udp_address(probes, bound_port).await {
        return Some(external);
    }
    discover_mapped_address(&transport).await
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        ADDRESS_PUBLISH_EVERY, Address, AddressListBoxed, AdnlKeyPair, DhtAddressPublisher,
        DhtUpdateRule, Int256, IpAddr, SeedPeer, SocketAddr, VALUE_TTL_SECS, address_dht_value,
        combine_publishers, is_globally_routable, known_candidates, lookup::xor_distance, now_i32,
        short_id, value_matches,
    };

    #[tokio::test]
    async fn combined_publisher_forwards_the_same_resolver_set_to_membership() {
        let seeds = vec![SeedPeer::from_public_key([7; 32], "8.8.8.8:5000")];
        let (address_tx, mut address_rx) = tokio::sync::mpsc::unbounded_channel();
        let (membership_tx, mut membership_rx) = tokio::sync::mpsc::unbounded_channel();
        let address: DhtAddressPublisher = Arc::new(move |nodes| {
            let address_tx = address_tx.clone();
            Box::pin(async move {
                address_tx.send(nodes).expect("test receiver stays open");
                1
            })
        });
        let membership: DhtAddressPublisher = Arc::new(move |nodes| {
            let membership_tx = membership_tx.clone();
            Box::pin(async move {
                membership_tx.send(nodes).expect("test receiver stays open");
                1
            })
        });

        let acknowledged = combine_publishers(address, Some(membership))(seeds.clone()).await;

        assert_eq!(acknowledged, 1);
        assert_eq!(address_rx.recv().await.unwrap(), seeds);
        assert_eq!(membership_rx.recv().await.unwrap(), seeds);
    }

    #[tokio::test]
    async fn combined_publisher_republishes_membership_every_round() {
        let address_calls = Arc::new(AtomicUsize::new(0));
        let membership_calls = Arc::new(AtomicUsize::new(0));
        let address: DhtAddressPublisher = {
            let calls = Arc::clone(&address_calls);
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { 1 })
            })
        };
        let membership: DhtAddressPublisher = {
            let calls = Arc::clone(&membership_calls);
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { 1 })
            })
        };
        let publish = combine_publishers(address, Some(membership));
        let seeds = vec![SeedPeer::from_public_key([7; 32], "8.8.8.8:5000")];

        for _ in 0..ADDRESS_PUBLISH_EVERY {
            publish(seeds.clone()).await;
        }
        assert_eq!(address_calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            membership_calls.load(Ordering::Relaxed),
            usize::try_from(ADDRESS_PUBLISH_EVERY).unwrap()
        );

        publish(seeds).await;
        assert_eq!(address_calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            membership_calls.load(Ordering::Relaxed),
            usize::try_from(ADDRESS_PUBLISH_EVERY).unwrap() + 1
        );
    }

    #[test]
    fn address_value_is_signed_and_parses_back() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let external: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let value = address_dht_value(&keypair, external);

        // Both signatures must verify against the published key.
        assert!(value.key.verify_signature(), "key description signature");
        assert!(value.verify_signature(), "value signature");

        // The value is keyed by this node's address key: the
        // raw ADNL short id, which the DHT hashes into the key
        // id its routing uses.
        assert_eq!(
            value.key.key.id,
            Int256(short_id(&keypair.public_key)),
            "the value must be keyed by the node's address key"
        );
        assert_eq!(value.key.key.name, b"address".to_vec());
        assert_eq!(value.key.key.idx, 0);
        assert!(matches!(value.key.update_rule, DhtUpdateRule::Signature));

        // The payload is the signed address list with our address.
        let address_list: AddressListBoxed = tl_proto::deserialize(&value.value).unwrap();
        assert_eq!(address_list.addrs.len(), 1);
        let expected_ip = u32::from(std::net::Ipv4Addr::new(8, 8, 8, 8)) as i32;
        assert!(matches!(
            address_list.addrs[0],
            Address::Udp { ip, port } if ip == expected_ip && port == 443
        ));
        assert!(address_list.version > 0);

        // The ttl stays inside the one hour the DHT accepts.
        let now = now_i32();
        assert!(
            value.ttl > now && value.ttl <= now + VALUE_TTL_SECS,
            "ttl {} must be within one hour of now {now}",
            value.ttl
        );
    }

    #[test]
    fn address_value_supports_ipv6() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let external: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let value = address_dht_value(&keypair, external);
        assert!(value.verify_signature());
        let address_list: AddressListBoxed = tl_proto::deserialize(&value.value).unwrap();
        assert!(matches!(
            address_list.addrs[0],
            Address::Udp6 { port, .. } if port == 443
        ));
    }

    #[test]
    fn xor_distance_orders_closest_first() {
        let target = [0xffu8; 32];
        let near = [0xf0u8; 32];
        let far = [0x00u8; 32];
        assert!(xor_distance(&near, &target) < xor_distance(&far, &target));
        // Distance is symmetric.
        assert_eq!(xor_distance(&near, &target), xor_distance(&target, &near));
    }

    #[test]
    fn route_probe_rejects_non_routable_addresses() {
        assert!(!is_globally_routable(
            "127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_globally_routable("10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(!is_globally_routable(
            "192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_globally_routable(
            "169.254.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(!is_globally_routable("::1".parse::<IpAddr>().unwrap()));
        assert!(!is_globally_routable("fc00::1".parse::<IpAddr>().unwrap()));
        assert!(is_globally_routable("8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn candidates_parse_from_seed_peers() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let seed = SeedPeer::from_public_key(keypair.public_key.to_bytes(), "8.8.8.8:5000");
        let candidates = known_candidates(&[seed]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, short_id(&keypair.public_key));
        assert_eq!(candidates[0].address.to_string(), "8.8.8.8:5000");
    }

    #[test]
    fn value_matches_only_the_current_published_address() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let external: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let value = address_dht_value(&keypair, external);
        assert!(value_matches(&value, &keypair, external));
        // The same node at another address is a stale value.
        assert!(!value_matches(
            &value,
            &keypair,
            "8.8.8.8:444".parse().unwrap()
        ));
        // Another identity's value never matches.
        let other = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        assert!(!value_matches(&value, &other, external));
    }

    #[test]
    fn value_matches_rejects_a_tampered_payload() {
        let keypair = AdnlKeyPair::generate(&mut rand::rngs::OsRng);
        let external: SocketAddr = "8.8.8.8:443".parse().unwrap();
        let mut value = address_dht_value(&keypair, external);
        // The signature covers the boxed unsigned bytes, so a payload
        // edit without re-signing must fail the signature check.
        value.value.push(0);
        assert!(!value_matches(&value, &keypair, external));
    }
}
