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
/// Several servers are queried because a single answer cannot be
/// distinguished from a symmetric NAT that maps the socket differently
/// per destination: such a mapping is not the address overlay peers can
/// deliver to.
const STUN_SERVERS: [&str; 3] = [
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// Distinct mapped addresses that must agree before one is reported.
///
/// Endpoint-independent mapping reports the same `ip:port` no matter
/// which server is asked; a symmetric NAT reports a different port per
/// server and is rejected.
const REQUIRED_AGREEMENT: usize = 2;

/// Per server budget for a binding exchange.
const STUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

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

/// Asks the STUN servers where the shared socket is reachable from.
///
/// One `BindingRequest` per server, sent from the transport's own
/// socket, so the reported mapping describes that socket.  A mapping is
/// reported only when [`REQUIRED_AGREEMENT`] servers describe the same
/// `ip:port` and the address is globally routable, see
/// [`is_publishable_ip`].  Otherwise the round is logged and no address
/// is published, which is the same outcome as before STUN existed.
pub(super) async fn discover_mapped_address(transport: &AdnlUdpTransport) -> Option<SocketAddr> {
    let mut observations: Vec<(SocketAddr, SocketAddr)> = Vec::new();
    for server in STUN_SERVERS {
        let Some(server) = resolve_stun_server(server).await else {
            log::debug!("dht address publish: stun server {server} did not resolve");
            continue;
        };
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
        log::debug!("dht address publish: stun server {server} reports {mapped} for this socket");
        if !is_publishable_ip(mapped) {
            log::debug!(
                "dht address publish: mapped address {mapped} is not globally routable, \
                 continuing"
            );
            continue;
        }
        if let Some((previous, _)) = observations.first()
            && *previous != mapped
        {
            log::debug!(
                "dht address publish: stun servers disagree ({previous} vs {mapped}); \
                 the NAT maps per destination, continuing"
            );
            continue;
        }
        observations.push((mapped, server));
        if observations.len() >= REQUIRED_AGREEMENT {
            log::info!(
                "dht address publish: stun reports {mapped} reachable for this socket \
                 ({server})"
            );
            return Some(mapped);
        }
    }
    log::debug!(
        "dht address publish: stun reached no agreement ({}/{} servers), no address \
         published",
        observations.len(),
        REQUIRED_AGREEMENT
    );
    None
}

/// Resolves a STUN server name, preferring an IPv4 address.
///
/// The binding exchange handled here is IPv4 only, so a name that
/// resolves to IPv6 alone is treated as unreachable.
async fn resolve_stun_server(server: &str) -> Option<SocketAddr> {
    lookup_host(server)
        .await
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
}
