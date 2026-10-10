//! Send half of an ADNL session.
//!
//! Packet encoding, pair-wide sequence bookkeeping, and the address list
//! stamping that tells the peer how to reach this node.  Both transport
//! variants send through [`Source::send_to`]: a solo socket is connected,
//! a shared socket is addressed per datagram.

use super::*;

impl SessionInner {
    /// Encodes and sends a packet, allocating the pair's sequence
    /// numbers.
    pub(super) async fn send_contents(&self, contents: PacketContents) -> Result<usize, AdnlError> {
        let mut state = self.state.lock().await;
        self.send_locked(&mut state, contents).await
    }

    /// Encodes and sends a packet with the session state lock held.
    pub(super) async fn send_locked(
        &self,
        state: &mut SessionState,
        contents: PacketContents,
    ) -> Result<usize, AdnlError> {
        self.note_sent_queries(state, &contents);
        if state.channels.is_empty() {
            return self.send_direct_locked(state, contents).await;
        }
        let contents = self.seal_seqno(state, contents);
        log::trace!(
            "send_channel: to={} seqno={:?} confirm={:?}",
            self.remote_addr.lock().unwrap(),
            contents.seqno,
            contents.confirm_seqno,
        );
        let packet = state
            .channels
            .first_mut()
            .expect("channel presence was checked above")
            .encode(contents)?;
        self.send_packet(&packet).await
    }

    /// Sends a raw packet to the peer's current source address.
    /// Sends a raw packet to the peer's current source address.
    async fn send_packet(&self, packet: &[u8]) -> Result<usize, AdnlError> {
        let addr = *self.remote_addr.lock().unwrap();
        self.source.send_to(packet, addr).await
    }

    pub(super) async fn send_direct_locked(
        &self,
        state: &mut SessionState,
        mut contents: PacketContents,
    ) -> Result<usize, AdnlError> {
        contents.from = Some(TlPublicKey::Ed25519 {
            key: tonutils_tl::Int256(self.local.public_key.to_bytes()),
        });
        contents.seqno = Some(next_outgoing_seqno(&self.remote_id));
        contents.confirm_seqno = Some(highest_received_seqno(&self.remote_id));
        contents.reinit_date.get_or_insert(local_reinit_date());
        contents
            .dst_reinit_date
            .get_or_insert(peer_reinit_date(&self.remote_id));
        self.fill_address(state, &mut contents);
        log::trace!(
            "send_direct: to={} seqno={:?} confirm={:?} reinit={:?} dst_reinit={:?}",
            self.remote_addr.lock().unwrap(),
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
        self.send_packet(&packet).await
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
    /// the long-lived session's address current.
    ///
    /// Ties are the remaining case: two sessions of the same ADNL node id
    /// that send inside the same second both advertise the same `version`,
    /// and the incumbent keeps the address. A lookup session therefore
    /// stamps one second ahead of the wall clock when
    /// [`AdnlUdpSession::set_transient_address`] is on, so it wins the
    /// exchange it is waiting for, and the live session's next keepalive
    /// takes the version lead back.
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
            let version = if self.transient_address.load(Ordering::Relaxed) {
                now_i32().saturating_add(1)
            } else {
                now_i32()
            };
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
    fn note_sent_queries(&self, state: &mut SessionState, contents: &PacketContents) {
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
    pub(super) fn log_stray_answers(&self, state: &SessionState, contents: &PacketContents) {
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
