//! Send half of an ADNL session.
//!
//! Packet encoding, pair-wide sequence bookkeeping, and the address list
//! stamping that tells the peer how to reach this node.  Both transport
//! variants send through [`Source::send_to`]: a solo socket is connected,
//! a shared socket is addressed per datagram.

use std::net::SocketAddr;

use crate::{
    adnl::{
        AdnlError, encrypt_direct, now_i32,
        tl::{AddressList, AdnlMessage, AdnlTlPublicKey as TlPublicKey, PacketContents},
        udp::{
            MAX_TRACKED_QUERIES, MAX_UDP_PACKET_SIZE, highest_received_seqno, local_reinit_date,
            next_outgoing_seqno, peer_reinit_date,
            session::{SessionInner, SessionState},
        },
    },
    tl::TlInt256 as Int256,
};

impl SessionInner {
    /// Encodes and sends a packet, allocating the pair's sequence
    /// numbers.
    pub(super) async fn send_contents(&self, contents: PacketContents) -> Result<usize, AdnlError> {
        let mut state = self.state.lock().await;
        self.send_locked(&mut state, contents, None).await
    }

    /// Sends an answer to an inbound query and logs how long that query
    /// waited inside this process since its datagram was read.
    ///
    /// Both [`AdnlUdpSession::send_answer`] and the receive task's
    /// registered fast answers go through here, so every answer is
    /// built and stamped identically and reported by the same
    /// diagnostic whatever answered it.
    pub(super) async fn send_answer_to(
        &self,
        query_id: &Int256,
        answer: Vec<u8>,
    ) -> Result<usize, AdnlError> {
        let result = self
            .send_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: Some(AdnlMessage::Answer {
                    query_id: query_id.clone(),
                    answer,
                }),
                messages: None,
                address: None,
                priority_address: None,
                seqno: None,
                confirm_seqno: None,
                recv_addr_list_version: None,
                recv_priority_addr_list_version: None,
                reinit_date: None,
                dst_reinit_date: None,
                signature: None,
                rand2: vec![0; 7],
            })
            .await;
        self.log_answer_latency(query_id);
        result
    }

    /// Encodes and sends one packet to `dest` instead of the session's
    /// adopted address.
    ///
    /// A probe dials candidate endpoints of a peer whose steady-state
    /// traffic must keep the established address: the peer keys its
    /// answers to the source address it saw last, so adopting a
    /// candidate would repoint the manager's keepalives and broadcasts
    /// at an endpoint that may be stale, and one concurrent sweep over
    /// the recorded endpoints could leave the whole session pinned to a
    /// dead one.  Everything else matches [`Self::send_contents`]; the
    /// adopted address is left untouched.
    #[cfg(feature = "overlay")]
    pub(super) async fn send_contents_to(
        &self,
        contents: PacketContents,
        dest: SocketAddr,
    ) -> Result<usize, AdnlError> {
        let mut state = self.state.lock().await;
        self.send_locked(&mut state, contents, Some(dest)).await
    }

    /// Encodes and sends a packet with the session state lock held.
    ///
    /// `dest` carries this packet to an explicit address; `None` sends
    /// to the session's adopted address.
    pub(super) async fn send_locked(
        &self,
        state: &mut SessionState,
        contents: PacketContents,
        dest: Option<SocketAddr>,
    ) -> Result<usize, AdnlError> {
        Self::note_sent_queries(state, &contents);
        if state.channels.is_empty() {
            return self.send_direct_locked(state, contents, dest).await;
        }
        let contents = self.seal_seqno(state, contents);
        let dest = self.destination(dest);
        log::trace!(
            "send_channel: to={dest} seqno={:?} confirm={:?}",
            contents.seqno,
            contents.confirm_seqno,
        );
        let packet = state
            .channels
            .first_mut()
            .expect("channel presence was checked above")
            .encode(contents)?;
        self.send_packet(&packet, dest).await
    }

    /// Resolves the address one packet is sent to: the explicit probe
    /// destination when given, the session's adopted address otherwise.
    fn destination(&self, dest: Option<SocketAddr>) -> SocketAddr {
        dest.unwrap_or_else(|| *self.remote_addr.lock().unwrap())
    }

    /// Sends a raw packet to `dest`.
    async fn send_packet(&self, packet: &[u8], dest: SocketAddr) -> Result<usize, AdnlError> {
        self.source.send_to(packet, dest).await
    }

    pub(super) async fn send_direct_locked(
        &self,
        state: &mut SessionState,
        mut contents: PacketContents,
        dest: Option<SocketAddr>,
    ) -> Result<usize, AdnlError> {
        contents.from = Some(TlPublicKey::Ed25519 {
            key: Int256(self.local.public_key.to_bytes()),
        });
        contents.seqno = Some(next_outgoing_seqno(&self.remote_id));
        contents.confirm_seqno = Some(highest_received_seqno(&self.remote_id));
        contents.reinit_date.get_or_insert(local_reinit_date());
        contents
            .dst_reinit_date
            .get_or_insert(peer_reinit_date(&self.remote_id));
        self.fill_address(state, &mut contents);
        let dest = self.destination(dest);
        log::trace!(
            "send_direct: to={dest} seqno={:?} confirm={:?} reinit={:?} dst_reinit={:?}",
            contents.seqno,
            contents.confirm_seqno,
            contents.reinit_date,
            contents.dst_reinit_date,
        );
        let mut unsigned = contents.clone();
        unsigned.signature = None;
        let signature = self.local.sign_raw(&tl_proto::serialize(unsigned));
        contents.signature = Some(signature.to_vec());
        let encrypted = encrypt_direct(&self.remote, &tl_proto::serialize(contents));
        let mut packet = Vec::with_capacity(32 + encrypted.len());
        packet.extend_from_slice(&self.remote_id);
        packet.extend_from_slice(&encrypted);
        if packet.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        self.send_packet(&packet, dest).await
    }

    /// Assigns the peer pair's shared sequence numbers to an outgoing
    /// packet and fills the address fields the peer needs to reach us.
    ///
    /// Direct and channel packets share one sequence number space per
    /// peer pair, so both paths allocate from [`super::PeerPairState`]
    /// here instead of from any session-local counter.
    fn seal_seqno(&self, state: &mut SessionState, contents: PacketContents) -> PacketContents {
        let mut contents = contents;
        contents.seqno = Some(next_outgoing_seqno(&self.remote_id));
        contents.confirm_seqno = Some(highest_received_seqno(&self.remote_id));
        self.fill_address(state, &mut contents);
        contents
    }

    /// Fills `address`/`recv_addr_list_version` on an outgoing packet.
    ///
    /// Upstream learns a peer's address only from `packet.addr_list()`
    /// (`AdnlPeerPairImpl::receive_packet_checked` in `adnl/adnl-peer.cpp`):
    /// an initialized but address-less `adnl.addressList` makes the
    /// receiver record the datagram's source address implicitly.  A peer
    /// that never learns our address cannot route anything *it* initiates
    /// to us - neither the `overlay.ping` that turns us into a verified
    /// overlay member nor the overlay broadcasts - which leaves the node
    /// invisible to the overlay even though its own queries are answered.
    ///
    /// `version` is the current wall-clock time, not a value frozen at
    /// connect time.  The receiver keeps a single source address per peer
    /// pair and replaces it only on a strictly greater `version`, so a
    /// frozen value makes the *first* session to reach a peer win
    /// forever.  Re-stamping each packet with the time it is sent keeps
    /// the long-lived session's address current, and one socket per node
    /// id means every stamp advertises the same, always-reachable
    /// address.
    ///
    /// `reinit_date` must equal the packet level reinit date
    /// ([`local_reinit_date`]): upstream compares it against the pair's
    /// own `reinit_date_` and calls `AdnlPeerPairImpl::reinit` when the
    /// announced date is greater, which resets the peer's sequence
    /// numbers and drops its ADNL channel.  Because that reinit is never
    /// signalled back, our replay window would then reject the peer's
    /// restarted sequence numbers and all later answers would be lost.
    fn fill_address(&self, state: &SessionState, contents: &mut PacketContents) {
        if contents.address.is_none() {
            let version = now_i32();
            contents.address = Some(AddressList {
                addrs: Vec::new(),
                version,
                reinit_date: local_reinit_date(),
                priority: 0,
                expire_at: 0,
            });
            self.note_stamped_address_version(version);
        }
        if contents.recv_addr_list_version.is_none() {
            contents.recv_addr_list_version = state.peer_addr_list_version;
        }
    }

    /// Remembers the query ids this session put on the wire.
    ///
    /// ADNL routes an answer to the source address the peer stored for
    /// this process's ADNL node id, and every session of that id shares
    /// the address, so an answer can arrive on a session that did not
    /// ask for it.  Recording the ids is what lets that be told apart
    /// from a peer that never replies.
    fn note_sent_queries(state: &mut SessionState, contents: &PacketContents) {
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Query { query_id, .. } = message {
                state.sent_queries.push_back(query_id.0);
            }
        }
        while state.sent_queries.len() > MAX_TRACKED_QUERIES {
            state.sent_queries.pop_front();
        }
    }

    /// Logs answers to queries this session never sent.
    ///
    /// Seeing this line while a sibling lookup session times out is the
    /// direct evidence that the peer answered for a query another
    /// session of the same ADNL node id is the one waiting for.
    pub(super) fn log_stray_answers(state: &SessionState, contents: &PacketContents) {
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Answer { query_id, .. } = message
                && !state.sent_queries.contains(&query_id.0)
            {
                log::debug!(
                    "ADNL UDP answer for a query this socket never sent: query_id={} (a sibling session of the same ADNL node id is the one waiting for it)",
                    query_id.to_hex(),
                );
            }
        }
    }
}
