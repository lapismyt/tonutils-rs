# ADNL UDP

ADNL UDP is required for general TON peer-to-peer networking, DHT, overlays,
and mempool scanning. It is not the same implementation path as ADNL TCP
liteserver connections.

## Expected Responsibilities

UDP ADNL must handle:

- datagram boundaries,
- peer address lists,
- packet contents flags,
- public key identity,
- signatures,
- channel creation and confirmation,
- reinit dates,
- sequence numbers,
- packet parts for large messages.

## Relevant TL Areas

`ton_api.tl` contains ADNL packet and message definitions such as:

- `adnl.packetContents`,
- `adnl.message.createChannel`,
- `adnl.message.confirmChannel`,
- `adnl.message.custom`,
- `adnl.message.query`,
- `adnl.message.answer`,
- `adnl.message.part`,
- `adnl.addressList`,
- `adnl.node`.

## Implementation Risks

- UDP packet loss and reordering.
- Large message fragmentation.
- NAT and address list freshness.
- Correct signature coverage.
- Interaction with DHT and overlay routing.

## Crate Design

The `udp` module is split by concern so no file outgrows the repository's
1000-line limit: `udp/mod.rs` holds the constants, the shared per-peer-pair
sequence/reinit state, and `AdnlUdpPeer`/`AdnlUdpSocket`; `udp/cipher.rs` holds
`AdnlChannelCipher`, `AdnlChannelPacket`, and the direct-packet crypto;
`udp/session.rs` holds the `AdnlUdpSession` transport state machine; and
`udp/session/query.rs` is a second inherent block with the DHT and overlay
query helpers. `udp/tests.rs` covers all four. Everything is re-exported
unchanged through `adnl/mod.rs`, so no public path moved.

`tonutils-adnl` exposes `AdnlUdpPeer`, `AdnlUdpSocket`, and authenticated
direct/channel packet primitives behind the opt-in `udp` feature. Direct
packets use the upstream layout of destination id, ephemeral Ed25519 key,
SHA-256 digest, and AES-CTR ciphertext. Established channel packets use the
channel id, canonical AES-channel digest/key/IV derivation, TL
`adnl.packetContents`, bounded sequence replay tracking, and ACK validation.
All datagram APIs enforce the 64 KiB bound and provide Tokio timeouts.

`AdnlUdpSession` expects the caller to provision the remote identity and now
supports create/confirm channel negotiation plus typed DHT and overlay query
helpers. A session keeps up to `MAX_SESSION_CHANNELS` (3) established
channels: the newest one sends, every held one is still accepted for decoding,
so a re-key does not discard datagrams the peer already put on the wire and a
channel negotiated by an earlier round keeps working after the peer switches
back to it. `create_channel_message` bundles `adnl.message.createChannel` with
the first outbound query (rate-limited to one request per 10 seconds) instead
of sending it as a standalone packet, which is how a session that never sees a
`confirmChannel` still gets its first query delivered. Fragmentation and NAT
traversal remain outside this layer.

An outgoing packet always carries an initialized, address-less
`adnl.addressList`: upstream learns a peer's source address only from
`packet.addr_list()` (`AdnlPeerPairImpl::receive_packet_checked`), so without
it the peer can never initiate anything back. `fill_address` stamps
`version` with the current wall-clock time on *every* packet rather than a
value frozen at connect time. The receiver keeps one source address per peer
pair and replaces it only on a strictly greater `version`, so a frozen value
would let the first session to reach a peer own that address forever: a
one-shot discovery, growth, or DHT lookup socket would then win, die, and
every `overlay.ping` and broadcast the peer sends would go to a closed port.
Re-stamping each packet lets the long-lived overlay session win the address
back on its next 10-second keepalive, while a transient session still wins
for as long as it is actually answering. Ties are the remaining case: two
sessions of the same ADNL node id that send inside the same second advertise
the same `version`, and the incumbent keeps the address, so a lookup socket
would never see the answer to its own query. A session marked with
`set_transient_address` therefore stamps `version` one second ahead of the
wall clock, wins that single exchange, and then yields the address to the
live session on its next keepalive - which bounds, rather than removes, the
window in which the peer may still address a dropped lookup socket.
`reinit_date` stays equal to the
packet-level local reinit date: a greater one makes the receiver call
`AdnlPeerPairImpl::reinit`, which resets its sequence numbers and drops the
ADNL channel without signalling us, after which our replay window rejects its
restarted sequence numbers.

## Missing Work

- Add packet fixtures from official nodes.
- Add deterministic simulated UDP tests.
