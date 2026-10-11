//! DHT `address` value publishing for the native UDP path.
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
//! the ADNL key pair, the DHT nodes closest to the key are located
//! with `dht.findNode`, and the value is stored with `dht.store`
//! on the closest of them, mirroring pytoniq's
//! `DhtClient::raw_store_value`.  Upstream DHT nodes accept a
//! store only when they are responsible for the key and drop it
//! otherwise, so storing on the closest known nodes is what makes
//! the value retrievable.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use tonutils_adnl::{AdnlAddress, AdnlUdpTransport, KeyPair, PublicKey as AdnlPublicKey, now_i32};
use tonutils_overlay::SeedPeer;
use tonutils_tl::Int256;
use tonutils_tl::tl::network::{
    Address, AddressListBoxed, DhtKey, DhtKeyDescription, DhtNode, DhtNodesBoxed, DhtUpdateRule,
    DhtValue, DhtValueResult, PublicKey as TlPublicKey,
};

use super::lookup::{dht_key_id, shared_session};

/// How many `dht.store` targets one publish round tries.
///
/// The nodes responsible for a key are the ones closest to it in
/// the DHT keyspace, and a store on a node that is not responsible
/// is dropped silently, so a handful of the closest known nodes is
/// enough - and keeps one round cheap.
const PUBLISH_TARGETS: usize = 4;

/// How many nodes one `dht.findNode` round asks for.
const FIND_NODE_K: i32 = 8;

/// Lifetime of a published `address` value.
///
/// Upstream sets `ttl = now + 3600` and the DHT rejects anything
/// longer than an hour plus a minute, so one hour is the maximum
/// useful lifetime.  The publishing task re-publishes well inside
/// it.
const VALUE_TTL_SECS: i32 = 3600;

/// Interval between publish rounds.
///
/// A published value lives for [`VALUE_TTL_SECS`], so re-publishing
/// every ten minutes keeps it fresh with a wide margin even when a
/// round fails.
pub const PUBLISH_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// A DHT node a publish round can store the value on.
#[derive(Clone)]
struct DhtCandidate {
    /// The node's position in the DHT keyspace: its ADNL short id.
    id: [u8; 32],
    public_key: AdnlPublicKey,
    address: SocketAddr,
}

/// The ADNL short id of a public key: the node's position in the
/// DHT keyspace, and the id its `address` value is keyed by.
fn short_id(public_key: &AdnlPublicKey) -> [u8; 32] {
    AdnlAddress::from(public_key).to_bytes()
}

/// XOR distance between two DHT key ids - the ordering the DHT
/// uses to pick the nodes responsible for a key.
fn xor_distance(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut distance = [0u8; 32];
    for i in 0..32 {
        distance[i] = a[i] ^ b[i];
    }
    distance
}

/// Builds the signed DHT `address` value for this node.
///
/// The value carries the externally reachable UDP address in an
/// `adnl.addressList`, signed by the ADNL key pair the same way
/// upstream signs it in `publish_address_list`: both the key
/// description and the value itself are signed over their boxed
/// serialization with the signature field still empty.
#[allow(clippy::large_types_passed_by_value)]
pub fn address_dht_value(local_keypair: &KeyPair, external: SocketAddr) -> DhtValue {
    let version = now_i32();
    let address_list = AddressListBoxed {
        addrs: vec![address_for(external)],
        version,
        reinit_date: tonutils_adnl::adnl::udp::local_reinit_date(),
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
                ip: tonutils_tl::tl::network::Int128(
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
        let socket = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            Ok(socket) => socket,
            Err(_) => continue,
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

/// Runs one publish round: locates the DHT nodes closest to this
/// node's `address` key and stores the signed value on the closest
/// of them.
///
/// Returns the number of `dht.stored` answers.  A single store on
/// a responsible node makes the value retrievable through
/// `dht.findValue`, which is what overlay peers use to resolve
/// this node's address before pinging it.
#[allow(clippy::large_types_passed_by_value)]
pub async fn publish_dht_address_value(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    external: SocketAddr,
    dht_nodes: &[SeedPeer],
    timeout: Duration,
) -> usize {
    let value = address_dht_value(&local_keypair, external);
    let key_id = dht_key_id(short_id(&local_keypair.public_key), b"address").0;
    let mut candidates = known_candidates(dht_nodes);
    enrich_with_find_node(local_addr, local_keypair, &key_id, &mut candidates, timeout).await;
    candidates.sort_by_key(|candidate| xor_distance(&candidate.id, &key_id));

    // The stores are independent of each other, so run them
    // concurrently instead of waiting for each round trip in turn.
    let target_count = candidates.len().min(PUBLISH_TARGETS);
    let stores = candidates.iter().take(target_count).map(|candidate| {
        let value = value.clone();
        async move {
            let session = match shared_session(
                local_addr,
                local_keypair,
                candidate.public_key,
                candidate.address,
            )
            .await
            {
                Ok(session) => session,
                Err(error) => {
                    log::debug!(
                        "dht address publish: session to {} failed: {error}",
                        candidate.address
                    );
                    return 0usize;
                }
            };
            if let Err(error) = session.dht_store(value, timeout).await {
                log::debug!(
                    "dht address publish: store on {} failed: {error}",
                    candidate.address
                );
                return 0;
            }
            1
        }
    });
    let stored: usize = futures::future::join_all(stores).await.into_iter().sum();
    log::debug!("dht address publish: stored on {stored} of {target_count} candidate node(s)");
    stored
}

/// The publish round's starting candidate set: the known DHT
/// nodes, in arbitrary order.  [`publish_dht_address_value`]
/// sorts them by distance to the key.
fn known_candidates(dht_nodes: &[SeedPeer]) -> Vec<DhtCandidate> {
    let mut candidates = Vec::with_capacity(dht_nodes.len());
    for node in dht_nodes {
        let Ok(address) = node.address.parse() else {
            continue;
        };
        let Some(public_key) = AdnlPublicKey::from_bytes(node.peer.as_bytes()) else {
            continue;
        };
        candidates.push(DhtCandidate {
            id: short_id(&public_key),
            public_key,
            address,
        });
    }
    candidates
}

/// Asks the closest known node for the DHT nodes closest to the
/// `address` key and merges them into `candidates`.
///
/// The known set is usually the static seed list, which is rarely
/// the part of the keyspace a fresh node id lands in; one
/// `findNode` round is what reaches the nodes that are actually
/// responsible for the key.
#[allow(clippy::large_types_passed_by_value)]
async fn enrich_with_find_node(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    key_id: &[u8; 32],
    candidates: &mut Vec<DhtCandidate>,
    timeout: Duration,
) {
    let Some(closest) = candidates
        .iter()
        .min_by(|a, b| xor_distance(&a.id, key_id).cmp(&xor_distance(&b.id, key_id)))
        .cloned()
    else {
        return;
    };
    let Ok(session) = shared_session(
        local_addr,
        local_keypair,
        closest.public_key,
        closest.address,
    )
    .await
    else {
        return;
    };
    let nodes = match session
        .dht_find_node(Int256(*key_id), FIND_NODE_K, timeout)
        .await
    {
        Ok(DhtNodesBoxed { nodes }) => nodes,
        Err(error) => {
            log::debug!(
                "dht address publish: findNode via {} failed: {error}",
                closest.address
            );
            return;
        }
    };
    let now = now_i32();
    let mut found = 0usize;
    for node in nodes {
        if !node.is_valid(now) {
            continue;
        }
        if let Some(candidate) = candidate_from_dht_node(&node)
            && !candidates
                .iter()
                .any(|existing| existing.id == candidate.id)
        {
            found += 1;
            candidates.push(candidate);
        }
    }
    if found > 0 {
        log::debug!(
            "dht address publish: findNode via {} returned {found} closer node(s)",
            closest.address
        );
    }
}

/// Extracts a publish candidate from a `dht.node` record.
fn candidate_from_dht_node(node: &DhtNode) -> Option<DhtCandidate> {
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

/// Verifies that the `address` value is retrievable from the DHT
/// after a publish round, logging the outcome.
///
/// This is the check that tells a store apart from a publish: a
/// `dht.stored` answer only means one node accepted the value,
/// while a `dht.valueFound` answer to `findValue` means the value
/// is reachable through the DHT, which is what overlay peers rely
/// on.
#[allow(clippy::large_types_passed_by_value)]
pub async fn verify_dht_address_value(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
    external: SocketAddr,
    dht_nodes: &[SeedPeer],
    timeout: Duration,
) -> bool {
    let key_id = dht_key_id(short_id(&local_keypair.public_key), b"address");
    let candidates = known_candidates(dht_nodes);
    let Some(closest) = candidates
        .iter()
        .min_by(|a, b| xor_distance(&a.id, &key_id.0).cmp(&xor_distance(&b.id, &key_id.0)))
        .cloned()
    else {
        return false;
    };
    let Ok(session) = shared_session(
        local_addr,
        local_keypair,
        closest.public_key,
        closest.address,
    )
    .await
    else {
        return false;
    };
    let result = match session.dht_find_value(key_id, 1, timeout).await {
        Ok(result) => result,
        Err(error) => {
            log::debug!(
                "dht address verify: findValue via {} failed: {error}",
                closest.address
            );
            return false;
        }
    };
    let DhtValueResult::Found { value } = result else {
        log::debug!("dht address verify: value for this node is not in the DHT yet");
        return false;
    };
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
    let published_address =
        address_list
            .addrs
            .iter()
            .any(|address| match (address, external.ip()) {
                (Address::Udp { ip, port }, IpAddr::V4(expected)) => {
                    *ip == u32::from(expected) as i32 && *port == i32::from(external.port())
                }
                _ => false,
            });
    if published_address {
        log::debug!("dht address verify: the DHT serves this node's address value");
    } else {
        log::debug!("dht address verify: the DHT serves a stale address value");
    }
    published_address
}

/// Publisher of this node's DHT `address` value for the native
/// UDP path.
///
/// One call runs a full publish round: it resolves the externally
/// reachable address, stores the signed value on the DHT nodes
/// closest to the key, verifies the value is served, and returns
/// how many nodes answered `dht.stored`.
pub type DhtAddressPublisher =
    Arc<dyn Fn(Vec<SeedPeer>) -> BoxFuture<'static, usize> + Send + Sync>;

/// Builds the publisher the native UDP path installs on the
/// scanner builder.
///
/// The publisher resolves the external address lazily on every
/// round: the address configured through
/// [`MempoolScannerBuilder::external_address`], then the
/// `TON_MEMPOOL_EXTERNAL_ADDRESS` environment variable, then a
/// route probe against the known DHT nodes.  A lazy resolution
/// keeps the value correct when the host's address changes
/// between rounds.
#[allow(clippy::large_types_passed_by_value)]
pub fn address_publisher(
    local_addr: SocketAddr,
    local_keypair: KeyPair,
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
            let stored =
                publish_dht_address_value(local_addr, local_keypair, external, &nodes, timeout)
                    .await;
            let served =
                verify_dht_address_value(local_addr, local_keypair, external, &nodes, timeout)
                    .await;
            log::info!("dht address publish: stored on {stored} node(s), served_by_dht={served}");
            stored
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
    local_keypair: &KeyPair,
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
    super::stun::discover_mapped_address(&transport).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_value_is_signed_and_parses_back() {
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
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
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
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
        let keypair = KeyPair::generate(&mut rand::rngs::OsRng);
        let seed = SeedPeer::from_public_key(keypair.public_key.to_bytes(), "8.8.8.8:5000");
        let candidates = known_candidates(&[seed]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, short_id(&keypair.public_key));
        assert_eq!(candidates[0].address.to_string(), "8.8.8.8:5000");
    }
}
