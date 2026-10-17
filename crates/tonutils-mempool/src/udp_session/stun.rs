//! Minimal RFC 5389 binding client for external address discovery.
//!
//! Behind NAT the route probe in [`super::publish`] reports a private
//! address, so the node never publishes its `dht.address` value and
//! peers reached only transitively cannot resolve where to ping it.
//! A STUN binding request observes the address the node's own socket
//! is reachable from.
//!
//! The request is sent from the scanner's shared ADNL socket through
//! [`AdnlUdpTransport::raw_exchange`], not from a fresh probe socket.
//! NAT maps each socket separately, so only the shared socket's own
//! mapping is the address peers can use; a probe socket's mapping
//! describes a port nothing else listens on.
//!
//! An observed mapping is evidence about where the socket is reachable
//! from the servers that answered, not proof that every overlay peer can
//! deliver to it.  [`discover_mapped_address`] therefore reports a
//! candidate and logs its reasoning rather than asserting reachability.
//!
//! A mapping is published only when servers behind distinct destinations
//! report the same `ip:port`: a symmetric NAT maps the socket separately
//! per destination, and such a per-destination port is not where other
//! peers can deliver.  Two hostnames that resolve to the same server are
//! one destination and their agreement proves nothing, so agreement is
//! counted across distinct server addresses only.  The server list is
//! swept more than once because one unanswered request says nothing
//! about the mapping.

use std::net::{Ipv4Addr, SocketAddr};

use tokio::net::lookup_host;
use tonutils_adnl::AdnlUdpTransport;

use super::publish::is_publishable_ip;

/// Magic cookie every RFC 5389 message carries, `0x2112A442`.
const MAGIC_COOKIE: u32 = 0x2112_A442;

/// `BindingRequest` message type.
const BINDING_REQUEST: u16 = 0x0001;

/// `BindingSuccessResponse` message type.
const BINDING_RESPONSE: u16 = 0x0101;

/// `MAPPED-ADDRESS` attribute type, RFC 5389 section 15.1.
const MAPPED_ADDRESS: u16 = 0x0001;

/// `XOR-MAPPED-ADDRESS` attribute type, RFC 5389 section 15.2.
const XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Public STUN servers queried for the socket's mapped address.
///
/// Several independent operators are queried because a mapping is only
/// publishable when servers behind distinct destinations report the same
/// `ip:port`: a symmetric NAT maps the socket separately per destination
/// and such a mapping is not the address overlay peers can deliver to.
/// The two Google hostnames are kept because they resolve to different
/// anycast addresses on some networks and deduplicate on others.
const STUN_SERVERS: [&str; 8] = [
    "stun.cloudflare.com:3478",
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.miwifi.com:3478",
    "stun.sipnet.net:3478",
    "stun.voip.blackberry.com:3478",
    "stun.ekiga.net:3478",
    "stun.nextcloud.com:443",
];

/// Distinct server addresses that must report the same mapping before
/// one is published.
///
/// An endpoint-independent NAT reports the same `ip:port` to every
/// destination; a symmetric NAT reports a different port per destination
/// and is rejected.  Two hostnames that resolve to one address are a
/// single destination, so only distinct addresses count.
const REQUIRED_AGREEMENT: usize = 2;

/// How many passes over [`STUN_SERVERS`] one discovery round makes.
///
/// A single unanswered request is not evidence of a per-destination NAT,
/// so a pass that reached fewer than [`REQUIRED_AGREEMENT`] agreements is
/// repeated for the servers that stayed silent; servers that already
/// answered are not asked again.
const STUN_SWEEPS: usize = 2;

/// Per server budget for a binding exchange.
const STUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Per name budget for resolving a STUN server.
const RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Builds a `BindingRequest` carrying `transaction_id`.
///
/// The message has no attributes: RFC 5389 makes every required part of
/// the binding query implicit in the header.
pub(super) fn binding_request(transaction_id: [u8; 12]) -> [u8; 20] {
    let mut request = [0u8; 20];
    request[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    request[2..4].copy_from_slice(&0u16.to_be_bytes());
    request[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    request[8..20].copy_from_slice(&transaction_id);
    request
}

/// Extracts the IPv4 mapped address from a `BindingSuccessResponse`.
///
/// Returns `None` for a response that is not the answer to the request
/// we sent, truncated where its header claims to end, or that carries no
/// IPv4 mapping.  A `MAPPED-ADDRESS` fallback is kept and only returned
/// when no `XOR-MAPPED-ADDRESS` follows, since the XOR form is the one
/// RFC 5389 mandates and the plain form is a legacy attribute.
pub(super) fn parse_binding_response(
    response: &[u8],
    transaction_id: [u8; 12],
) -> Option<SocketAddr> {
    if response.len() < 20 {
        return None;
    }
    if u16::from_be_bytes([response[0], response[1]]) != BINDING_RESPONSE {
        return None;
    }
    let message_len = u16::from_be_bytes([response[2], response[3]]) as usize;
    if response.len() < 20 + message_len {
        return None;
    }
    if response[4..8] != MAGIC_COOKIE.to_be_bytes() {
        return None;
    }
    if response[8..20] != transaction_id {
        return None;
    }
    let mut plain = None;
    let mut offset = 20;
    let end = 20 + message_len;
    while offset + 4 <= end {
        let attribute = u16::from_be_bytes([response[offset], response[offset + 1]]);
        let length = u16::from_be_bytes([response[offset + 2], response[offset + 3]]) as usize;
        let value = response.get(offset + 4..offset + 4 + length)?;
        // Attributes are padded to a four byte boundary without counting
        // the padding in their length, RFC 5389 section 15.
        offset = (offset + 4 + length + 3) & !3;
        let Some(mapped) = parse_address_attribute(attribute, value) else {
            continue;
        };
        if attribute == XOR_MAPPED_ADDRESS {
            return Some(mapped);
        }
        plain = Some(mapped);
    }
    plain
}

/// Decodes one address attribute, IPv4 only.
///
/// The IPv6 form of `XOR-MAPPED-ADDRESS` XORs the address against the
/// magic cookie followed by the transaction id, RFC 5389 section 15.2,
/// which this scanner does not need: it publishes IPv4 `dht.address`
/// values.
fn parse_address_attribute(attribute: u16, value: &[u8]) -> Option<SocketAddr> {
    if attribute != MAPPED_ADDRESS && attribute != XOR_MAPPED_ADDRESS {
        return None;
    }
    if value.len() < 8 || value[1] != 0x01 {
        return None;
    }
    let port = u16::from_be_bytes([value[2], value[3]]);
    let address = u32::from_be_bytes(value[4..8].try_into().ok()?);
    let (port, address) = if attribute == XOR_MAPPED_ADDRESS {
        (port ^ (MAGIC_COOKIE >> 16) as u16, address ^ MAGIC_COOKIE)
    } else {
        (port, address)
    };
    Some(SocketAddr::new(Ipv4Addr::from(address).into(), port))
}

/// Bookkeeping for one discovery round: which server destinations have
/// answered and whether their mappings agree.
#[derive(Default)]
struct MappingAgreement {
    answered: Vec<SocketAddr>,
    candidate: Option<SocketAddr>,
    agreed: usize,
    disagreed: bool,
}

impl MappingAgreement {
    fn has_answered(&self, server: SocketAddr) -> bool {
        self.answered.contains(&server)
    }

    /// Marks a server that answered with an unusable mapping (for
    /// example a non-routable address) as asked.
    fn mark_answered(&mut self, server: SocketAddr) {
        if !self.has_answered(server) {
            self.answered.push(server);
        }
    }

    /// Records a publishable mapping from `server` and reports whether
    /// [`REQUIRED_AGREEMENT`] distinct servers now agree on one address.
    ///
    /// The first mapping becomes the candidate; any different mapping
    /// from a distinct destination disqualifies the round, because a
    /// mapping that varies per destination is not where other peers can
    /// deliver.
    fn record_mapping(&mut self, server: SocketAddr, mapped: SocketAddr) -> bool {
        if self.has_answered(server) {
            return self.reached_agreement();
        }
        self.mark_answered(server);
        match self.candidate {
            None => {
                self.candidate = Some(mapped);
                self.agreed = 1;
            }
            Some(candidate) if candidate == mapped => self.agreed += 1,
            Some(_) => self.disagreed = true,
        }
        self.reached_agreement()
    }

    fn reached_agreement(&self) -> bool {
        !self.disagreed && self.agreed >= REQUIRED_AGREEMENT
    }

    fn candidate(&self) -> Option<SocketAddr> {
        self.candidate
    }

    fn disagreed(&self) -> bool {
        self.disagreed
    }

    fn agreed(&self) -> usize {
        self.agreed
    }
}

/// Asks the STUN servers where the shared socket is reachable from.
///
/// One `BindingRequest` per distinct server address, sent from the
/// transport's own socket, so the reported mapping describes that
/// socket.  A mapping is reported only when [`REQUIRED_AGREEMENT`]
/// distinct servers describe the same `ip:port` and the address is
/// globally routable, see [`is_publishable_ip`]; any disagreement from a
/// distinct destination disqualifies the round, and a round that reached
/// too few agreements is swept once more for the servers that stayed
/// silent.  Otherwise the round is logged and no address is published,
/// which is the correct outcome behind a per-destination NAT.
pub(super) async fn discover_mapped_address(transport: &AdnlUdpTransport) -> Option<SocketAddr> {
    let mut agreement = MappingAgreement::default();
    let mut sweeps_run = 0;
    for sweep in 1..=STUN_SWEEPS {
        sweeps_run = sweep;
        for name in STUN_SERVERS {
            let Some(server) = resolve_stun_server(name).await else {
                log::debug!("dht address publish: stun server {name} did not resolve");
                continue;
            };
            if agreement.has_answered(server) {
                continue;
            }
            let transaction_id = tonutils_tl::Int256::random().0[..12].try_into().unwrap();
            let request = binding_request(transaction_id);
            let response = match transport.raw_exchange(server, &request, STUN_TIMEOUT).await {
                Ok(response) => response,
                Err(error) => {
                    log::debug!("dht address publish: binding request to {server} failed: {error}");
                    continue;
                }
            };
            let Some(mapped) = parse_binding_response(&response, transaction_id) else {
                log::debug!(
                    "dht address publish: binding response from {server} did not parse \
                     ({} bytes)",
                    response.len()
                );
                continue;
            };
            log::debug!(
                "dht address publish: stun server {server} reports {mapped} for this socket"
            );
            if !is_publishable_ip(mapped) {
                log::debug!(
                    "dht address publish: mapped address {mapped} is not globally routable, \
                     continuing"
                );
                agreement.mark_answered(server);
                continue;
            }
            if agreement.record_mapping(server, mapped) {
                log::info!(
                    "dht address publish: stun reports {mapped} reachable for this socket \
                     ({server}, sweep {sweep})"
                );
                return Some(mapped);
            }
            if agreement.disagreed()
                && let Some(candidate) = agreement.candidate()
            {
                log::debug!(
                    "dht address publish: stun servers disagree ({candidate} vs {mapped}); \
                     the NAT maps per destination, continuing"
                );
            }
        }
        if agreement.disagreed() {
            // Distinct destinations already reported different mappings;
            // retrying silent servers cannot make the mapping endpoint
            // independent.
            break;
        }
    }
    log::debug!(
        "dht address publish: stun reached no agreement ({}/{} servers after sweep {}), \
         no address published",
        agreement.agreed(),
        REQUIRED_AGREEMENT,
        sweeps_run
    );
    None
}

/// Resolves a STUN server name, preferring an IPv4 address.
///
/// The binding exchange handled here is IPv4 only, so a name that
/// resolves to IPv6 alone is treated as unreachable.  Resolution is
/// bounded so a stuck resolver cannot stall the publish round.
async fn resolve_stun_server(server: &str) -> Option<SocketAddr> {
    tokio::time::timeout(RESOLVE_TIMEOUT, lookup_host(server))
        .await
        .ok()?
        .ok()?
        .find(|address| address.is_ipv4())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSACTION_ID: [u8; 12] = [0x11; 12];

    /// Encodes a `BindingSuccessResponse` for one address attribute.
    ///
    /// Written field by field from RFC 5389 rather than captured from a
    /// live server, so the test checks the parser against the
    /// specification instead of against one server's output.
    fn binding_response(attribute: u16, value: [u8; 8]) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        message.extend_from_slice(&12u16.to_be_bytes());
        message.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        message.extend_from_slice(&TRANSACTION_ID);
        message.extend_from_slice(&attribute.to_be_bytes());
        message.extend_from_slice(&8u16.to_be_bytes());
        message.extend_from_slice(&value);
        message
    }

    /// Encodes the address `SocketAddr` into an attribute's value bytes.
    fn address_value(mapped: SocketAddr, xor: bool) -> [u8; 8] {
        let SocketAddr::V4(address) = mapped else {
            panic!("test vectors are IPv4");
        };
        let mut value = [0u8; 8];
        value[1] = 0x01;
        let port = if xor {
            address.port() ^ (MAGIC_COOKIE >> 16) as u16
        } else {
            address.port()
        };
        let raw = u32::from(*address.ip());
        let raw = if xor { raw ^ MAGIC_COOKIE } else { raw };
        value[2..4].copy_from_slice(&port.to_be_bytes());
        value[4..8].copy_from_slice(&raw.to_be_bytes());
        value
    }

    #[test]
    fn request_carries_header_and_transaction_id() {
        let request = binding_request(TRANSACTION_ID);
        assert_eq!(request.len(), 20, "header has no attributes");
        assert_eq!(
            u16::from_be_bytes([request[0], request[1]]),
            BINDING_REQUEST
        );
        assert_eq!(u16::from_be_bytes([request[2], request[3]]), 0);
        assert_eq!(
            u32::from_be_bytes(request[4..8].try_into().unwrap()),
            MAGIC_COOKIE
        );
        assert_eq!(request[8..20], TRANSACTION_ID);
    }

    #[test]
    fn xor_mapped_address_is_decoded() {
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let response = binding_response(XOR_MAPPED_ADDRESS, address_value(mapped, true));
        assert_eq!(
            parse_binding_response(&response, TRANSACTION_ID),
            Some(mapped)
        );
    }

    #[test]
    fn legacy_mapped_address_is_decoded_without_xor() {
        let mapped: SocketAddr = "198.51.100.9:19302".parse().unwrap();
        let response = binding_response(MAPPED_ADDRESS, address_value(mapped, false));
        assert_eq!(
            parse_binding_response(&response, TRANSACTION_ID),
            Some(mapped)
        );
    }

    #[test]
    fn xor_form_wins_over_the_legacy_attribute() {
        let xor: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let plain: SocketAddr = "198.51.100.9:19302".parse().unwrap();
        let mut response = binding_response(MAPPED_ADDRESS, address_value(plain, false));
        response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        response.extend_from_slice(&8u16.to_be_bytes());
        response.extend_from_slice(&address_value(xor, true));
        response[2..4].copy_from_slice(&24u16.to_be_bytes());
        assert_eq!(parse_binding_response(&response, TRANSACTION_ID), Some(xor));
    }

    #[test]
    fn padded_attributes_are_walked_without_bleeding_into_the_next() {
        // A one byte attribute pads to four bytes on the wire without
        // counting the padding in its length, RFC 5389 section 15.
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let mut response = Vec::new();
        response.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
        response.extend_from_slice(&16u16.to_be_bytes());
        response.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        response.extend_from_slice(&TRANSACTION_ID);
        response.extend_from_slice(&0x0008u16.to_be_bytes());
        response.extend_from_slice(&1u16.to_be_bytes());
        response.push(0xAA);
        response.extend_from_slice(&[0, 0, 0]);
        response.extend_from_slice(&XOR_MAPPED_ADDRESS.to_be_bytes());
        response.extend_from_slice(&8u16.to_be_bytes());
        response.extend_from_slice(&address_value(mapped, true));
        assert_eq!(
            parse_binding_response(&response, TRANSACTION_ID),
            Some(mapped)
        );
    }

    #[test]
    fn response_for_another_transaction_is_rejected() {
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let response = binding_response(XOR_MAPPED_ADDRESS, address_value(mapped, true));
        assert_eq!(parse_binding_response(&response, [0x22; 12]), None);
    }

    #[test]
    fn truncated_response_is_rejected() {
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let response = binding_response(XOR_MAPPED_ADDRESS, address_value(mapped, true));
        for cut in [0, 8, 19, 24, 31] {
            assert_eq!(
                parse_binding_response(&response[..cut], TRANSACTION_ID),
                None,
                "a response truncated to {cut} bytes must not parse"
            );
        }
    }

    #[test]
    fn attribute_past_the_declared_length_is_rejected() {
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let mut response = binding_response(XOR_MAPPED_ADDRESS, address_value(mapped, true));
        // Shrink the attribute length so its value runs past where the
        // message declares its own end.
        response[22..24].copy_from_slice(&64u16.to_be_bytes());
        assert_eq!(parse_binding_response(&response, TRANSACTION_ID), None);
    }

    #[test]
    fn non_ipv4_address_attributes_are_ignored() {
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let mut value = address_value(mapped, true);
        value[1] = 0x02;
        let response = binding_response(XOR_MAPPED_ADDRESS, value);
        assert_eq!(parse_binding_response(&response, TRANSACTION_ID), None);
    }

    #[test]
    fn agreement_needs_distinct_server_addresses() {
        let mut agreement = MappingAgreement::default();
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let server: SocketAddr = "198.51.100.9:3478".parse().unwrap();
        assert!(
            !agreement.record_mapping(server, mapped),
            "one destination is not agreement"
        );
        assert!(
            !agreement.record_mapping(server, mapped),
            "the same destination twice is still one observation"
        );
        assert_eq!(agreement.agreed(), 1);
        let other: SocketAddr = "198.51.100.10:3478".parse().unwrap();
        assert!(
            agreement.record_mapping(other, mapped),
            "two distinct destinations reporting the same mapping agree"
        );
    }

    #[test]
    fn a_different_mapping_from_another_destination_blocks_agreement() {
        let mut agreement = MappingAgreement::default();
        let first: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        let second: SocketAddr = "203.0.113.8:3478".parse().unwrap();
        let a: SocketAddr = "198.51.100.9:3478".parse().unwrap();
        let b: SocketAddr = "198.51.100.10:3478".parse().unwrap();
        let c: SocketAddr = "198.51.100.11:3478".parse().unwrap();
        assert!(!agreement.record_mapping(a, first));
        assert!(
            !agreement.record_mapping(b, second),
            "disagreement is not agreement"
        );
        assert!(agreement.disagreed());
        assert!(
            !agreement.record_mapping(c, first),
            "a later matching answer cannot outvote a recorded disagreement"
        );
    }

    #[test]
    fn answered_servers_are_not_asked_again() {
        let mut agreement = MappingAgreement::default();
        let server: SocketAddr = "198.51.100.9:3478".parse().unwrap();
        let mapped: SocketAddr = "203.0.113.7:3478".parse().unwrap();
        assert!(!agreement.has_answered(server));
        agreement.mark_answered(server);
        assert!(agreement.has_answered(server));
        assert!(
            !agreement.record_mapping(server, mapped),
            "an unusable answer still counts the destination as asked"
        );
        assert_eq!(agreement.agreed(), 0);
    }
}
