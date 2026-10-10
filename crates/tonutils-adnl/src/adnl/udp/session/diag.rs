//! Receive-side diagnostics of an ADNL session.
//!
//! Kept as a second inherent block next to [`super::query`]: the facts logged
//! here are observable only while a datagram is being processed, and they are
//! what a live run needs to tell *why* a peer stays silent.
//!
//! Two questions are answered, both without changing any behaviour:
//!
//! * **Which socket did the peer address?**  Upstream keeps one source address
//!   per ADNL node id and echoes the version it currently holds back in every
//!   packet as `recv_addr_list_version`
//!   (`AdnlPeerPairImpl::send_packet` in `adnl/adnl-peer.cpp`).  Comparing that
//!   with the highest version this process stamped for the same peer shows
//!   whether the address the peer will use belongs to a long-lived session or
//!   to a lookup socket that is about to be dropped.
//! * **What did the peer ask?**  A query that lands on a one-shot lookup socket
//!   is never answered - the wait loop only looks for a matching `Answer` - so
//!   an `overlay.ping` received there leaves no trace in the overlay counters.

use super::*;

use tl_proto::TlRead;
use tonutils_tl::tl::network::OverlayQuery;

impl SessionInner {
    /// Logs every receive-side signal that explains a silent peer.
    pub(super) fn log_recv_diagnostics(&self, contents: &PacketContents) {
        self.log_peer_address_view(contents);
        self.log_inbound_queries(contents);
    }

    /// Records the `adnl.addressList.version` stamped on an outgoing
    /// packet.
    ///
    /// Called from [`SessionInner::fill_address`] so the receive-side
    /// comparison below knows what this process last announced and from
    /// which local port.  Every session of one node id shares the
    /// transport's socket, so the recorded port is the single port the
    /// peer can address this node on.
    pub(super) fn note_stamped_address_version(&self, version: i32) {
        let socket = self.source.local_addr().map_or(0, |addr| addr.port());
        note_our_addr_version(
            &self.remote_id,
            version,
            socket,
            self.transient_address
                .load(std::sync::atomic::Ordering::Relaxed),
        );
    }

    /// Compares the address version the peer reports with the one we sent.
    ///
    /// `ours` is the highest version ever stamped for this peer, which
    /// includes stamps made by sibling lookup sockets of the same ADNL node
    /// id, and `owner` is the local port of the socket that stamp came from.
    ///
    /// Read together with the socket this packet arrived on, three outcomes
    /// are possible and each points at a different failure:
    ///
    /// * equal and same port - the peer is talking to this socket;
    /// * equal but different port - the peer moved to a sibling socket, which
    ///   for a long-lived session means a dropped lookup socket owns the
    ///   address the peer will now use; the logged owner kind says which;
    /// * smaller - the peer never saw our newest stamp, so that packet was
    ///   rejected or lost and the peer still uses an older address.
    ///
    /// `None` means the peer echoed no `recv_addr_list_version`, which per
    /// upstream `AdnlPeerPairImpl::send_messages_from_queue` corresponds to an
    /// empty `addr_list_`.  Live runs show peers doing this while still sending
    /// packets, so treat it as "the peer reports no address list for us" rather
    /// than as proof that the peer cannot reach us.
    fn log_peer_address_view(&self, contents: &PacketContents) {
        let socket = self
            .source
            .local_addr()
            .map_or_else(|_| "?".into(), |address| address.to_string());
        let kind = if self
            .transient_address
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            "lookup"
        } else {
            "live"
        };
        let Some(theirs) = contents.recv_addr_list_version else {
            log::debug!(
                "ADNL peer holds no address list for us: socket={kind} self={socket} peer={:?} {}",
                self.remote_id,
                packet_shape(contents),
            );
            return;
        };
        let (ours, owner, owner_lookup) = our_addr_view(&self.remote_id);
        let mine = self.source.local_addr().map_or(0, |address| address.port());
        let owner_kind = if owner_lookup { "lookup" } else { "live" };
        if theirs == ours && owner == mine {
            log::debug!(
                "ADNL address view: peer={:?} socket={kind} holds={theirs} owner=this socket={socket} {}",
                self.remote_id,
                packet_shape(contents),
            );
        } else {
            log::debug!(
                "ADNL address view mismatch: peer={:?} holds={theirs} ours={ours} owner={owner_kind}:{owner} this_socket={kind}:{socket} {}",
                self.remote_id,
                packet_shape(contents),
            );
        }
    }

    /// Logs every inbound `adnl.message.query` together with its TL ids.
    ///
    /// The overlay wraps its queries in `overlay.query overlay:int256`, so the
    /// inner id - `overlay.ping`, `overlay.getRandomPeers`,
    /// `overlay.getRandomPeersV2` - is what identifies the traffic, and the
    /// socket kind says whether anyone is listening for it.
    ///
    /// Answers are logged as well, keyed by peer, because a query and an
    /// answer exercise different directions of the same pair: comparing who
    /// answers us with who holds an address list for us separates "the peer
    /// never saw us" from "the peer saw us but does not originate anything".
    fn log_inbound_queries(&self, contents: &PacketContents) {
        let socket_kind = if self
            .transient_address
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            "lookup"
        } else {
            "live"
        };
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Answer { query_id, .. } = message {
                log::debug!(
                    "ADNL inbound answer: socket={socket_kind} peer={:?} query_id={}",
                    self.remote_id,
                    query_id.to_hex(),
                );
                continue;
            }
            let AdnlMessage::Query { query_id, query } = message else {
                continue;
            };
            let mut wrapped: Option<(u32, String)> = None;
            let mut rest: &[u8] = query;
            if let Ok(OverlayQuery::Query { overlay }) = OverlayQuery::read_from(&mut rest) {
                let inner = peek_u32(rest);
                wrapped = inner.map(|id| (id, hex::encode(overlay.0)));
            }
            match wrapped {
                Some((inner, overlay)) => log::debug!(
                    "ADNL inbound overlay query: socket={socket_kind} peer={:?} query_id={} inner=0x{inner:08x} overlay={overlay} len={}",
                    self.remote_id,
                    query_id.to_hex(),
                    query.len(),
                ),
                None => log::debug!(
                    "ADNL inbound query: socket={socket_kind} peer={:?} query_id={} id={} len={}",
                    self.remote_id,
                    query_id.to_hex(),
                    peek_u32(query).map_or_else(|| "?".into(), |id| format!("0x{id:08x}")),
                    query.len(),
                ),
            }
        }
    }
}

/// Compact description of the flag-based fields of one received packet.
///
/// Every field below rides its own bit of `flags:#`, so seeing which of them
/// are present says two things at once: whether other optional fields decode
/// correctly next to `recv_addr_list_version` (a decoder fault would clear
/// *all* of them), and whether the datagram was direct.  Upstream only calls
/// `packet.set_reinit_date(...)` under `if (!via_channel)`
/// (`AdnlPeerPairImpl::send_messages_from_queue`), so an inbound packet that
/// carries `reinit_date` travelled unprotected while one without it came over
/// an established ADNL channel.
fn packet_shape(contents: &PacketContents) -> String {
    let their_address = contents.address.as_ref().map_or_else(
        || "-".to_string(),
        |address| {
            format!(
                "v{}+{}addr(p{})",
                address.version,
                address.addrs.len(),
                address.priority
            )
        },
    );
    format!(
        "shape:recv_prio={:?} their_addr={their_address} seqno={} confirm={} reinit={:?} dst_reinit={:?}",
        contents.recv_priority_addr_list_version,
        contents.seqno.is_some(),
        contents.confirm_seqno.is_some(),
        contents.reinit_date,
        contents.dst_reinit_date,
    )
}

/// Reads the leading TL constructor id of `bytes`.
fn peek_u32(bytes: &[u8]) -> Option<u32> {
    let head: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(head))
}
