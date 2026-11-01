//! Shared registry of overlay member endpoints, populated by the native UDP
//! path and consumed by the QUIC discovery path.
//!
//! Upstream TON runs QUIC in `validator-engine` (validators and full nodes),
//! never in `dht-server`: `QuicSender::add_local_id_coro` binds a `QuicServer`
//! on the node's own ADNL address with `NODE_PORT_OFFSET = 1000`
//! (`quic/quic-sender.h`), and `full-node-fast-sync-overlays.cpp` starts it
//! unconditionally ("Enable quic server even if use_quic is not set"). The
//! global config's `dht.static_nodes` - the scanner's bootstrap seeds - are
//! `dht-server` deployments, so QUIC probes against them time out on every
//! port. The nodes that do answer QUIC are overlay members, which the UDP
//! discovery path already resolves, so this registry hands their endpoints to
//! [`quic_overlay_lookup_with_endpoints`](crate::overlay::quic_overlay_lookup_with_endpoints).
//!
//! `QuicSender::get_ip_address` prefers an explicitly advertised
//! `adnl.address.quic` entry from the peer's address list and only falls back
//! to `adnl_port + 1000` when none is present; [`member_quic_target`] mirrors
//! that preference order.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use crate::overlay::PeerId;

/// Upper bound on registered members.  Beyond it, inserts of *new* members are
/// ignored (existing entries keep updating), which keeps the registry's memory
/// and the QUIC probe fan-out bounded while the overlay pool grows.
const MAX_REGISTERED_MEMBERS: usize = 256;

/// How a QUIC probe target's endpoint was obtained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuicEndpointSource {
    /// The member's DHT `address` record advertised an `adnl.address.quic`
    /// entry - upstream `QuicSender::get_ip_address` prefers it.
    Advertised,
    /// Derived as `adnl_udp_port + QUIC_PORT_OFFSET`, upstream
    /// `QuicSender::NODE_PORT_OFFSET = 1000`, the fallback for members whose
    /// address record carries no QUIC endpoint.
    UdpPortOffset,
}

impl QuicEndpointSource {
    /// Short label used in probe logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Advertised => "advertised",
            Self::UdpPortOffset => "udp+1000",
        }
    }
}

/// Endpoints known for one overlay member.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemberEndpoint {
    /// The member's `adnl.address.udp` endpoint, when resolved.
    pub udp: Option<SocketAddr>,
    /// The member's advertised `adnl.address.quic` endpoint, when its DHT
    /// `address` record carried one.
    pub quic: Option<SocketAddr>,
}

/// Shared registry mapping an overlay member to its endpoints.
///
/// A plain [`Mutex`] is enough: guards are never held across an `.await`.
pub type OverlayMemberEndpoints = Arc<Mutex<HashMap<PeerId, MemberEndpoint>>>;

/// Records the member's ADNL UDP endpoint, keeping any advertised QUIC one.
///
/// Ignored once `MAX_REGISTERED_MEMBERS` distinct members are registered, so
/// a long-running scanner cannot grow the map without bound.
pub fn record_member(endpoints: &OverlayMemberEndpoints, peer: PeerId, udp: SocketAddr) {
    let Ok(mut map) = endpoints.lock() else {
        return;
    };
    if let Some(entry) = map.get_mut(&peer) {
        entry.udp = Some(udp);
        return;
    }
    if map.len() >= MAX_REGISTERED_MEMBERS {
        log::trace!("record_member: registry is full, ignoring {peer:?}");
        return;
    }
    map.insert(peer, MemberEndpoint::default());
    if let Some(entry) = map.get_mut(&peer) {
        entry.udp = Some(udp);
    }
}

/// Records a QUIC endpoint advertised by the member's DHT `address` record.
///
/// Insert-or-update like [`record_member`]: a member seen only through its
/// advertised QUIC endpoint is registered without a UDP endpoint.
pub fn record_member_quic(endpoints: &OverlayMemberEndpoints, peer: PeerId, quic: SocketAddr) {
    let Ok(mut map) = endpoints.lock() else {
        return;
    };
    if let Some(entry) = map.get_mut(&peer) {
        entry.quic = Some(quic);
        return;
    }
    if map.len() >= MAX_REGISTERED_MEMBERS {
        log::trace!("record_member_quic: registry is full, ignoring {peer:?}");
        return;
    }
    map.insert(
        peer,
        MemberEndpoint {
            udp: None,
            quic: Some(quic),
        },
    );
}

/// The QUIC endpoint to dial for `peer`, preferring an advertised
/// `adnl.address.quic` and falling back to `fallback_udp` plus `offset`.
#[must_use]
pub fn member_quic_target(
    endpoints: &OverlayMemberEndpoints,
    peer: PeerId,
    fallback_udp: SocketAddr,
    offset: u16,
) -> (SocketAddr, QuicEndpointSource) {
    let advertised = endpoints
        .lock()
        .ok()
        .and_then(|map| map.get(&peer).copied())
        .and_then(|entry| entry.quic);
    match advertised {
        Some(quic) => (quic, QuicEndpointSource::Advertised),
        None => (
            SocketAddr::new(fallback_udp.ip(), fallback_udp.port().wrapping_add(offset)),
            QuicEndpointSource::UdpPortOffset,
        ),
    }
}

/// Snapshot of the registered members as probe seeds: `(peer, udp)` for
/// members with a UDP endpoint, falling back to the advertised QUIC endpoint
/// for members known only through `adnl.address.quic`.  The consumer passes
/// the address to [`member_quic_target`], which re-reads any advertised QUIC
/// entry, so the final dialled endpoint is correct either way.
#[must_use]
pub fn known_member_seeds(endpoints: &OverlayMemberEndpoints) -> Vec<(PeerId, SocketAddr)> {
    let Ok(map) = endpoints.lock() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(peer, entry)| Some((*peer, entry.udp.or(entry.quic)?)))
        .collect()
}

/// Number of registered members.
#[must_use]
pub fn registered_member_count(endpoints: &OverlayMemberEndpoints) -> usize {
    endpoints.lock().map(|map| map.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_REGISTERED_MEMBERS, OverlayMemberEndpoints, PeerId, QuicEndpointSource, SocketAddr,
        known_member_seeds, member_quic_target, record_member, record_member_quic,
        registered_member_count,
    };

    fn peer(byte: u8) -> PeerId {
        PeerId::from_bytes([byte; 32])
    }

    fn addr(port: u16) -> SocketAddr {
        format!("1.2.3.4:{port}").parse().unwrap()
    }

    #[test]
    fn derived_target_adds_the_upstream_port_offset() {
        let endpoints = OverlayMemberEndpoints::default();
        let (target, source) = member_quic_target(&endpoints, peer(1), addr(54390), 1000);
        assert_eq!(target, addr(55390));
        assert_eq!(source, QuicEndpointSource::UdpPortOffset);
    }

    #[test]
    fn advertised_target_wins_over_the_port_offset() {
        let endpoints = OverlayMemberEndpoints::default();
        record_member(&endpoints, peer(1), addr(54390));
        record_member_quic(&endpoints, peer(1), addr(22222));
        let (target, source) = member_quic_target(&endpoints, peer(1), addr(54390), 1000);
        assert_eq!(target, addr(22222));
        assert_eq!(source, QuicEndpointSource::Advertised);
    }

    #[test]
    fn recording_udp_keeps_an_existing_advertised_quic() {
        let endpoints = OverlayMemberEndpoints::default();
        record_member_quic(&endpoints, peer(1), addr(22222));
        record_member(&endpoints, peer(1), addr(54390));
        let entry = endpoints.lock().unwrap().get(&peer(1)).copied().unwrap();
        assert_eq!(entry.udp, Some(addr(54390)));
        assert_eq!(entry.quic, Some(addr(22222)));
    }

    #[test]
    fn known_member_seeds_expose_udp_and_advertised_fallback() {
        let endpoints = OverlayMemberEndpoints::default();
        record_member(&endpoints, peer(1), addr(1000));
        record_member(&endpoints, peer(2), addr(2000));
        record_member_quic(&endpoints, peer(2), addr(22222));
        // Member known only through an advertised QUIC entry still yields a
        // seed: its advertised endpoint stands in for the UDP one.
        record_member_quic(&endpoints, peer(3), addr(33333));
        let mut seeds = known_member_seeds(&endpoints);
        seeds.sort_by_key(|(peer, ..)| peer.as_bytes()[0]);
        assert_eq!(seeds.len(), 3);
        assert_eq!(seeds[0], (peer(1), addr(1000)));
        assert_eq!(seeds[1], (peer(2), addr(2000)));
        assert_eq!(seeds[2], (peer(3), addr(33333)));
        // Feeding a seed back through member_quic_target keeps the advertised
        // preference: peer 2 dials its QUIC port, not udp + offset.
        let (target, source) = member_quic_target(&endpoints, peer(2), addr(2000), 1000);
        assert_eq!(
            (target, source),
            (addr(22222), QuicEndpointSource::Advertised)
        );
    }

    #[test]
    fn registry_stops_accepting_new_members_at_the_cap() {
        let endpoints = OverlayMemberEndpoints::default();
        // The loop registers every all-equal byte key `[i; 32]` for i in
        // 0..256, exactly MAX_REGISTERED_MEMBERS distinct members.
        for index in 0..MAX_REGISTERED_MEMBERS {
            record_member(
                &endpoints,
                PeerId::from_bytes([u8::try_from(index).expect("registry cap is below 256"); 32]),
                addr(1000),
            );
        }
        let mut late = [7u8; 32];
        late[31] = 8;
        let late = PeerId::from_bytes(late);
        record_member(&endpoints, late, addr(9999));
        assert_eq!(registered_member_count(&endpoints), MAX_REGISTERED_MEMBERS);
        assert!(
            endpoints.lock().unwrap().get(&late).is_none(),
            "members beyond the cap must not be registered"
        );
        // Existing entries keep updating past the cap.
        record_member(&endpoints, PeerId::from_bytes([0; 32]), addr(1234));
        let entry = endpoints
            .lock()
            .unwrap()
            .get(&PeerId::from_bytes([0; 32]))
            .copied()
            .unwrap();
        assert_eq!(entry.udp, Some(addr(1234)));
    }
}
