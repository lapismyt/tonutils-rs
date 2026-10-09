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

## Upstream Rules Verified From Source

These are the rules a peer applies to *our* packets, taken from
`adnl/adnl-peer.cpp` in `ton-blockchain/ton`. They decide whether a peer can
originate anything back to this node, so they are the reason an otherwise
healthy query can still leave `pongs_sent` at zero.

- **One address per peer pair, newest version wins.**
  `AdnlPeerPairImpl::receive_packet_checked` copies `packet.addr_list()` into
  `update_addr_list` whenever the field is present, *even when `addrs` is
  empty*: an empty address list is replaced by the datagram's source address
  (`addr_list.add_udp_adnl_address(packet.remote_addr())`). A packet without
  the field at all changes nothing. "Present" here is decided by
  `AdnlAddressList::empty()`, which is `version_ == -1` and **not** an address
  count - so an initialized list with zero addresses is a list, gets the
  implicit source address added, and still passes
  `AdnlPacket::run_basic_checks`, which only rejects `flags & f_address` when
  `addr_.empty()`. This is what makes an address-less client reachable: it
  never advertises a routable `AdnlLocalId::addr_list_`, so
  `update_packet` leaves `packet.addr_list()` unset, and the source address of
  the datagram is the only thing a peer can learn.
- **Replacement is strictly greater.** `update_addr_list` returns early when
  `(priority ? priority_addr_list_ : addr_list_).version() >= addr_list.version()`,
  so a second socket advertising the same second never displaces the incumbent
  and its own query is answered to the *other* socket. The address is written
  together with its `conns_`, so an accepted list both refreshes the echo
  above and keeps the peer able to originate traffic.
- **`reinit_date` is checked in two places, and an old value is fatal.**
  `receive_packet_checked` drops a packet outright when
  `reinit_date > now + 60`, when it is positive and *older* than the pair's
  `reinit_date_`, and when `dst_reinit_date > 0` is older than the receiver's
  own `Adnl::adnl_start_time()` (that last branch still applies the address
  list, then replies with a `nop`). `update_addr_list` repeats the first two
  checks: a newer date calls `reinit()`, an older one rejects the list.
  `reinit()` resets both sequence numbers and the channel but deliberately
  keeps `addr_list_`. The value this crate sends is `local_reinit_date()`, the
  process start time, so it is constant and only the very first packet
  performs a reinit.
- **An idle pair forgets us.** `send_packet_continue` schedules
  `drop_addr_list_at_` once nothing has been received for 9 minutes, and
  `get_conn()` then clears `addr_list_` and `conns_` on the next send. Until
  that point a peer that has our address keeps it, so a 5-minute run can
  never trip this rule - but it is the one mechanism that *does* erase an
  address without the peer doing anything wrong.
- **The peer echoes its view back, and the flag says whether it has one.**
  Every outgoing packet carries
  `recv_addr_list_version = addr_list_.version()` when `addr_list_` is
  initialized (`set_received_addr_list_version` sets `f_recv_addr_version`,
  `0x100`), i.e. the version of the address the peer currently holds for
  *us*. The field being **absent** means `version_ == -1`, i.e. the peer holds
  no address for this node and `get_conn()` would fail with
  `no active connections` - it can answer what we send it, but it can never
  originate an `overlay.ping` or a broadcast. A value that does not match
  what this process last stamped means the peer is addressing a different
  socket. Both readings are logged by `udp/session/diag.rs` together with the
  sibling flag fields (`recv_prio`, `seqno`, `reinit_date`) so a decoder fault
  can be told apart from a genuine absence.
- **`reinit_date` marks direct packets.** `send_messages_from_queue` calls
  `packet.set_reinit_date(...)` only under `if (!via_channel)`, so an inbound
  packet that carries `reinit_date`/`dst_reinit_date` (flag `0x400`) travelled
  unprotected while one without it came over an established ADNL channel.
  Flag bits, for reference: `f_address 0x10`, `f_seqno 0x40`,
  `f_confirm_seqno 0x80`, `f_recv_addr_version 0x100`,
  `f_recv_priority_addr_version 0x200`, `f_reinit_date 0x400`,
  `f_signature 0x800` - identical to this crate's `#[tl(flags_bit = ...)]`
  mapping.
- **A ready channel carries everything.**
  `AdnlPeerPairImpl::send_messages_from_queue` computes
  `bool via_channel = channel_ready_ && !try_reinit;` and that covers query
  answers too. Once the pair negotiated a channel, a *different* socket of the
  same ADNL node id receives channel packets it cannot decrypt: it holds no
  channel, so `decode_packet` rejects the prefix and the query runs to its
  timeout. This is why a fresh lookup socket to an already-connected member
  fails while the same lookup against a never-contacted seed succeeds.

`AdnlUdpSession::fill_address` restamps `version` on every packet for exactly
the first rule, and `set_transient_address` advertises `now + 1` so a one-shot
lookup socket can win its single exchange against a live session that stamped
the same second. The receive-side signals for all of this are logged by
`udp/session/diag.rs`: the echoed `recv_addr_list_version` next to the version
this process last stamped, and the local port of the socket that stamped it.

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
