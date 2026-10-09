//! State machine for one authenticated ADNL session over UDP.
//!
//! The transport half lives here; the query half (`dht_find_node`,
//! `overlay_get_random_peers`, and friends) is a second inherent block in
//! [`session::query`] so both files stay readable.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::Duration;

use tonutils_tl::tl::network::{AddressList, DhtMessage, PacketContents, PublicKey as TlPublicKey};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::crypto::{KeyPair, PublicKey};
use crate::{AdnlAddress, AdnlError};

use super::{
    AdnlChannelPacket, MAX_SESSION_CHANNELS, MAX_TRACKED_QUERIES, MAX_UDP_PACKET_SIZE,
    REQUEST_CHANNEL_INTERVAL, channel_id_for_secret, decrypt_direct, encrypt_direct,
    highest_received_seqno, local_reinit_date, message_kind, message_vector, next_outgoing_seqno,
    note_peer_reinit_date, now_i32, ordered_channel_ciphers, outgoing_seqno, peer_reinit_date,
    record_received_seqno,
};

mod query;

/// Authenticated UDP ADNL endpoint for direct packets and established channels.
pub struct AdnlUdpSession {
    socket: tokio::net::UdpSocket,
    local: KeyPair,
    remote: PublicKey,
    local_id: [u8; 32],
    remote_id: [u8; 32],
    /// Negotiated ADNL channels for this peer, newest first.
    ///
    /// The first entry is used for sending.  Earlier channels stay so that a
    /// re-key does not discard datagrams the peer already put on the wire,
    /// and so that a channel negotiated by an earlier round of this session
    /// keeps working after the peer switches back to it.
    channels: Vec<AdnlChannelPacket>,
    /// Local ADNL channel key for this session.  It is generated once and
    /// reused for every `adnl.message.createChannel` /
    /// `adnl.message.confirmChannel` this session sends, so repeated
    /// negotiation rounds stay idempotent instead of re-keying a channel the
    /// peer has already accepted.
    local_channel: KeyPair,
    /// Whether this session may establish ADNL channels.  One-shot sessions
    /// (DHT/overlay lookups) disable it: they are dropped right after the
    /// query, and a peer that keeps the resulting channel would send packets
    /// this process can no longer decrypt.
    confirm_channels: bool,
    /// Whether this session belongs to a one-shot lookup rather than to a
    /// long-lived peer session.
    ///
    /// The peer keeps exactly one source address per ADNL node id and replaces
    /// it only on a strictly greater `adnl.addressList.version`, so a lookup
    /// socket that ties with the node's live session never receives the answer
    /// to its own query.  Marked sessions therefore advertise a version one
    /// second ahead of the wall clock and win that single exchange;
    /// the live session takes the address back on its next keepalive.
    transient_address: bool,
    /// Peer channel public key the current channel was derived from.
    peer_channel_key: Option<[u8; 32]>,
    /// Date of the last channel request sent by this session, used to rate
    /// limit re-key attempts.
    channel_requested_at: Option<std::time::Instant>,
    /// When set, this session has sent `createChannel` and awaits the peer's
    /// `confirmChannel`.  The value is the date that was sent.
    pending_channel: Option<i32>,
    /// Sequence numbers of the peer pair itself are process-wide, see
    /// [`PeerPairState`]; only the local replay window is session state.
    received: VecDeque<u64>,
    /// Version of the address list last received from the peer, echoed back
    /// as `recv_addr_list_version` so the peer knows we keep its address.
    peer_addr_list_version: Option<i32>,
    /// Ids of the `adnl.message.query` packets this socket has sent, oldest
    /// first, so an answer addressed to a sibling session of the same ADNL
    /// node id can be told apart from a reply to this socket.
    sent_queries: std::collections::VecDeque<[u8; 32]>,
}

impl AdnlUdpSession {
    pub async fn connect(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local: KeyPair,
        remote: PublicKey,
    ) -> Result<Self, AdnlError> {
        let socket = tokio::net::UdpSocket::bind(local_addr).await?;
        socket.connect(remote_addr).await?;
        Ok(Self {
            socket,
            local_id: AdnlAddress::from(&local.public_key).to_bytes(),
            remote_id: AdnlAddress::from(&remote).to_bytes(),
            local,
            remote,
            channels: Vec::new(),
            local_channel: KeyPair::generate(&mut rand::rngs::OsRng),
            confirm_channels: true,
            transient_address: false,
            peer_channel_key: None,
            channel_requested_at: None,
            pending_channel: None,
            received: VecDeque::new(),
            peer_addr_list_version: None,
            sent_queries: VecDeque::new(),
        })
    }

    pub async fn connect_with_channel(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local: KeyPair,
        remote: PublicKey,
        timeout: Duration,
    ) -> Result<Self, AdnlError> {
        let mut session = Self::connect(local_addr, remote_addr, local, remote).await?;
        session.establish_channel(timeout).await?;
        Ok(session)
    }

    /// Enables or disables ADNL channel establishment for this session.
    ///
    /// Sessions used for a single query should pass `false`: the session is
    /// dropped right after the answer, while the peer keeps the negotiated
    /// channel for as long as it holds our ADNL node id.  Every later session
    /// would then receive channel packets it cannot decrypt.
    ///
    /// Defaults to `true`.
    pub fn set_confirm_channels(&mut self, enabled: bool) {
        self.confirm_channels = enabled;
    }

    /// Marks this session as a one-shot lookup socket.
    ///
    /// A lookup session answers one query and is then dropped, but it shares
    /// this process's ADNL node id with every live peer session. The peer keeps
    /// exactly one source address per node id and replaces it only on a
    /// strictly greater `adnl.addressList.version`, so a lookup socket that
    /// ties with a live session never receives the answer to its own query: the
    /// peer replies to the live socket instead and the lookup runs to its
    /// timeout. That is what makes a growth query to an already-connected
    /// overlay member silently produce nothing.
    ///
    /// A marked session advertises a version one second ahead of the wall
    /// clock and therefore wins that single exchange. The live session's next
    /// keepalive carries a later timestamp again and reclaims the address, so
    /// the window in which the peer may address a dropped lookup socket is
    /// bounded by the keepalive interval instead of being permanent.
    ///
    /// Defaults to `false`.
    pub fn set_transient_address(&mut self, enabled: bool) {
        self.transient_address = enabled;
    }

    /// Starts ADNL channel negotiation without waiting for the answer.
    ///
    /// Sends `adnl.message.createChannel` so that the peer confirms a channel
    /// this session can actually decrypt.  A session that starts without one
    /// is otherwise permanently deaf: the peer keeps using the channel it
    /// negotiated with an earlier session of the same ADNL node id, whose
    /// local channel key this session no longer holds, and every datagram it
    /// sends is dropped as an unknown prefix.
    ///
    /// Unlike [`Self::establish_channel`] this does not block, so a caller can
    /// bundle its first query with the handshake instead of paying a round
    /// trip before any application traffic.
    ///
    /// Does nothing for one-shot sessions (see [`Self::set_confirm_channels`])
    /// and when a channel is already established; repeated attempts are rate
    /// limited to once per ten seconds.
    pub async fn initiate_channel(&mut self) -> Result<(), AdnlError> {
        let Some(message) = self.create_channel_message() else {
            return Ok(());
        };
        self.send_direct_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(message),
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
        .await?;
        Ok(())
    }

    /// Assigns the peer pair's shared sequence numbers to an outgoing packet
    /// and fills the address fields the peer needs to reach us.
    ///
    /// Direct and channel packets share one sequence number space per peer
    /// pair, so both paths allocate from [`PeerPairState`] here instead of
    /// from any session-local counter.
    fn seal_seqno(&self, mut contents: PacketContents) -> PacketContents {
        contents.seqno = Some(next_outgoing_seqno(&self.remote_id));
        contents.confirm_seqno = Some(highest_received_seqno(&self.remote_id));
        self.fill_address(&mut contents);
        contents
    }

    /// Fills `address`/`recv_addr_list_version` on an outgoing packet.
    ///
    /// Upstream learns a peer's address only from `packet.addr_list()`
    /// (`AdnlPeerPairImpl::receive_packet_checked` in `adnl/adnl-peer.cpp`):
    /// an initialized but address-less `adnl.addressList` makes the receiver
    /// record the datagram's source address implicitly.  A peer that never
    /// learns our address cannot route anything *it* initiates to us -
    /// neither the `overlay.ping` that turns us into a verified overlay
    /// member nor the overlay broadcasts - which leaves the node invisible
    /// to the overlay even though its own queries are answered.
    ///
    /// `version` is the current wall-clock time, not a value frozen at
    /// connect time.  The receiver keeps a single source address per peer
    /// pair and replaces it only on a strictly greater `version`, so a frozen
    /// value makes the *first* session to reach a peer win forever: a later
    /// one-shot lookup socket - seed discovery, peer growth, a DHT hop - then
    /// owns that address for good, dies, and every `overlay.ping` and
    /// broadcast the peer sends afterwards goes to a closed port.  Re-stamping
    /// each packet with the time it is sent lets the long-lived overlay
    /// session win the address back on its next keepalive, while a transient
    /// session still wins for as long as it is actually answering.
    ///
    /// Ties are the remaining case: two sessions of the same ADNL node id
    /// that send inside the same second both advertise the same `version`, and
    /// the incumbent keeps the address. A lookup socket therefore stamps one
    /// second ahead of the wall clock when `set_transient_address` is on, so
    /// it wins the exchange it is waiting for, and the live session's next
    /// keepalive takes the address back.
    ///
    /// `reinit_date` must equal the packet level reinit date
    /// ([`local_reinit_date`]): upstream compares it against the pair's own
    /// `reinit_date_` and calls `AdnlPeerPairImpl::reinit` when the announced
    /// date is greater, which resets the peer's sequence numbers and drops its
    /// ADNL channel.  Because that reinit is never signalled back, our replay
    /// window would then reject the peer's restarted sequence numbers and all
    /// later answers would be lost.
    fn fill_address(&self, contents: &mut PacketContents) {
        if contents.address.is_none() {
            contents.address = Some(AddressList {
                addrs: Vec::new(),
                version: if self.transient_address {
                    now_i32().saturating_add(1)
                } else {
                    now_i32()
                },
                reinit_date: local_reinit_date(),
                priority: 0,
                expire_at: 0,
            });
        }
        if contents.recv_addr_list_version.is_none() {
            contents.recv_addr_list_version = self.peer_addr_list_version;
        }
    }

    /// Remembers the query ids this socket put on the wire.
    ///
    /// ADNL routes an answer to the source address the peer stored for this
    /// process's ADNL node id, and every session of that id shares the address,
    /// so an answer can arrive on a socket that did not ask for it. Recording
    /// the ids is what lets that be told apart from a peer that never replies.
    fn note_sent_queries(&mut self, contents: &PacketContents) {
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Query { query_id, .. } = message {
                self.sent_queries.push_back(query_id.0);
            }
        }
        while self.sent_queries.len() > MAX_TRACKED_QUERIES {
            self.sent_queries.pop_front();
        }
    }

    /// Logs answers to queries this socket never sent.
    ///
    /// Seeing this line while a sibling lookup session times out is the direct
    /// evidence that the peer answered on the wrong socket of the same ADNL
    /// node id, which is the failure mode `set_transient_address` exists for.
    fn log_stray_answers(&self, contents: &PacketContents) {
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Answer { query_id, .. } = message
                && !self.sent_queries.contains(&query_id.0)
            {
                log::debug!(
                    "ADNL UDP answer for a query this socket never sent: query_id={} (a sibling session of the same ADNL node id is the one waiting for it)",
                    query_id.to_hex()
                );
            }
        }
    }

    pub async fn send_contents(&mut self, contents: PacketContents) -> Result<usize, AdnlError> {
        self.note_sent_queries(&contents);
        if self.channels.is_empty() {
            return self.send_direct_contents(contents).await;
        }
        let contents = self.seal_seqno(contents);
        log::trace!(
            "send_channel: to={} seqno={:?} confirm={:?}",
            self.socket
                .peer_addr()
                .map_or_else(|_| "?".into(), |address| address.to_string()),
            contents.seqno,
            contents.confirm_seqno,
        );
        let packet = self
            .channels
            .first_mut()
            .expect("channel presence was checked above")
            .encode(contents)?;
        Ok(self.socket.send(&packet).await?)
    }

    pub async fn send_answer(
        &mut self,
        query_id: Int256,
        answer: Vec<u8>,
    ) -> Result<usize, AdnlError> {
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Answer { query_id, answer }),
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
        .await
    }

    async fn send_direct_contents(
        &mut self,
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
        self.fill_address(&mut contents);
        log::trace!(
            "send_direct: to={} seqno={:?} confirm={:?} reinit={:?} dst_reinit={:?}",
            self.socket
                .peer_addr()
                .map_or_else(|_| "?".into(), |address| address.to_string()),
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
        Ok(self.socket.send(&packet).await?)
    }

    /// Applies upstream's peer-reinit rules to an accepted packet.
    ///
    /// Mirrors `AdnlPeerPairImpl::receive_packet_checked` and
    /// `AdnlPeerPairImpl::reinit` in `adnl/adnl-peer.cpp`: the first announced
    /// `reinit_date` is only recorded, a *later* one resets the per-peer state
    /// (sequence numbers and the ADNL channel), and a packet carrying an older
    /// date is dropped as stale.
    ///
    /// Returns `false` when the packet must be ignored.
    fn apply_peer_reinit(&mut self, reinit_date: Option<i32>) -> bool {
        let date = reinit_date.unwrap_or(0);
        if note_peer_reinit_date(&self.remote_id, date) {
            self.reset_peer_state();
            log::debug!(
                "ADNL peer reinitialized: remote_id={} reinit_date={date}",
                hex::encode(self.remote_id)
            );
        }
        let current = peer_reinit_date(&self.remote_id);
        if date > 0 && date < current {
            log::trace!(
                "recv_contents: dropping stale packet (reinit_date={date}, peer_reinit_date={current})",
            );
            return false;
        }
        true
    }

    /// Drops state that only belongs to the peer's previous epoch, mirroring
    /// `AdnlPeerPairImpl::reinit`.
    fn reset_peer_state(&mut self) {
        self.received.clear();
        self.channels.clear();
        self.pending_channel = None;
    }

    #[allow(clippy::unnecessary_join)]
    pub async fn recv_contents(&mut self) -> Result<PacketContents, AdnlError> {
        const MAX_CONSECUTIVE_FAILURES: u32 = 256;
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        let mut consecutive_failures: u32 = 0;
        loop {
            let size = self.socket.recv(&mut packet).await?;
            if size > MAX_UDP_PACKET_SIZE {
                return Err(AdnlError::TooLongPacket);
            }
            let Some(contents) = self
                .decode_packet(&packet[..size], &mut consecutive_failures)
                .await?
            else {
                continue;
            };
            // Per-peer checks shared by direct and channel packets, mirroring
            // upstream `AdnlPeerPairImpl::receive_packet_checked`.  ADNL keeps
            // one sequence number space per peer pair, so both transports are
            // validated together here.
            if !self.apply_peer_reinit(contents.reinit_date) {
                continue;
            }
            if let Some(seqno) = contents.seqno {
                let highest = highest_received_seqno(&self.remote_id);
                if seqno == 0
                    || self.received.contains(&seqno)
                    || (highest > 4096 && seqno + 4096 < highest)
                {
                    log::trace!(
                        "recv_contents: seqno dropped (seqno={seqno}, highest={highest}, received_len={})",
                        self.received.len()
                    );
                    Self::count_failure(&mut consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
                    continue;
                }
                record_received_seqno(&self.remote_id, seqno);
                self.received.push_back(seqno);
                while self.received.len() > 4096 {
                    self.received.pop_front();
                }
            }
            if let Some(address) = &contents.address
                && self
                    .peer_addr_list_version
                    .is_none_or(|current| address.version > current)
            {
                self.peer_addr_list_version = Some(address.version);
            }
            if self.process_channel_control(&contents).await.is_err() {
                Self::count_failure(&mut consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
                continue;
            }
            self.log_stray_answers(&contents);
            log::trace!(
                "recv: from={} seqno={:?} confirm={:?} reinit={:?} kind={} messages={}",
                self.socket
                    .peer_addr()
                    .map_or_else(|_| "?".into(), |address| address.to_string()),
                contents.seqno,
                contents.confirm_seqno,
                contents.reinit_date,
                message_kind(contents.message.as_ref()),
                contents
                    .messages
                    .as_deref()
                    .map_or_else(|| "-".into(), message_vector),
            );
            return Ok(contents);
        }
    }

    /// Counts one rejected datagram and fails the session once too many
    /// consecutive datagrams could not be processed.
    fn count_failure(consecutive_failures: &mut u32, limit: u32) -> Result<(), AdnlError> {
        *consecutive_failures = consecutive_failures.saturating_add(1);
        if *consecutive_failures >= limit {
            return Err(AdnlError::IntegrityError);
        }
        Ok(())
    }

    /// Decodes one received datagram into `adnl.packetContents`.
    ///
    /// Returns `None` when the datagram was rejected.  Rejections are counted
    /// through `consecutive_failures`, and hitting the limit aborts the
    /// session with [`AdnlError::IntegrityError`].
    ///
    /// Sequence numbers are deliberately *not* validated here: they are
    /// validated once in [`Self::recv_contents`] together with the peer's
    /// `reinit_date`, exactly like upstream does after a packet has been
    /// decrypted, because direct and channel packets share the same sequence
    /// number space.  The channel still performs its own replay guard for
    /// standalone use, seeded from the shared peer pair counters below.
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        consecutive_failures: &mut u32,
    ) -> Result<Option<PacketContents>, AdnlError> {
        const MAX_CONSECUTIVE_FAILURES: u32 = 256;
        let size = packet.len();
        let channel_index = self.channels.iter().position(|channel| {
            size >= channel.inbound_id.len()
                && packet[..channel.inbound_id.len()] == channel.inbound_id
        });
        if let Some(index) = channel_index {
            let channel = &mut self.channels[index];
            channel.next_seqno = outgoing_seqno(&self.remote_id);
            channel.highest_seqno = highest_received_seqno(&self.remote_id);
            match channel.decode(packet) {
                Ok(contents) => {
                    return Ok(Some(contents));
                }
                Err(error) => {
                    log::debug!("dropping invalid ADNL channel packet: {error}");
                }
            }
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            if *consecutive_failures == MAX_CONSECUTIVE_FAILURES / 2 {
                log::warn!(
                    "ADNL channel degraded: {consecutive_failures} consecutive decode failures"
                );
            }
            if *consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                log::warn!(
                    "ADNL channel killed after {consecutive_failures} consecutive decode failures"
                );
            }
            return Ok(None);
        }
        if size < self.local_id.len() || packet[..self.local_id.len()] != self.local_id {
            let channel_ids = if self.channels.is_empty() {
                String::new()
            } else {
                format!(
                    " in_id={} out_id={}",
                    hex::encode(self.channels[0].inbound_id),
                    hex::encode(self.channels[0].outbound_id)
                )
            };
            if self.confirm_channels && self.channels.is_empty() {
                // The prefix is neither ours nor any channel this session
                // holds, so the peer is still using a channel negotiated by an
                // earlier session of ours - our local channel key changed with
                // it, so it can never be decrypted here.  Ask the peer to
                // re-key; `request_channel` rate limits itself to one attempt
                // per ten seconds.
                if let Err(error) = self.request_channel().await {
                    log::trace!("ADNL channel request failed: {error}");
                }
            }
            log::debug!(
                "recv_contents: dropping packet (size={size}, prefix={} local_id={}{channel_ids})",
                hex::encode(&packet[..32.min(size)]),
                hex::encode(self.local_id),
            );
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        }
        let Ok((_, payload)) = decrypt_direct(&self.local, &packet[32..size]) else {
            log::trace!("recv_contents: decrypt_direct failed for packet of size {size}");
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        };
        let Ok(contents) = tl_proto::deserialize::<PacketContents>(&payload) else {
            log::trace!(
                "recv_contents: PacketContents deserialization failed, payload len={}",
                payload.len()
            );
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        };
        let sender_from_from = contents.from.as_ref().and_then(|pk| match pk {
            TlPublicKey::Ed25519 { key } => PublicKey::from_bytes(key.0),
            _ => None,
        });
        if let Some(ref sender) = sender_from_from
            && sender != &self.remote
        {
            log::trace!(
                "recv_contents: sender key mismatch (expected={:?}, got={:?})",
                self.remote,
                sender
            );
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        }
        if let Some(from_short) = &contents.from_short
            && from_short.id.0 != self.remote_id
        {
            log::trace!("recv_contents: from_short id mismatch");
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        }
        let Some(signature) = &contents.signature else {
            log::trace!("recv_contents: missing signature");
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        };
        let Ok(signature) = signature.as_slice().try_into() else {
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        };
        let mut unsigned = contents.clone();
        unsigned.signature = None;
        if !self
            .remote
            .verify_raw(&tl_proto::serialize(unsigned), &signature)
        {
            log::trace!("recv_contents: signature verification failed");
            Self::count_failure(consecutive_failures, MAX_CONSECUTIVE_FAILURES)?;
            return Ok(None);
        }
        Ok(Some(contents))
    }

    async fn process_channel_control(
        &mut self,
        contents: &PacketContents,
    ) -> Result<(), AdnlError> {
        let messages = contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten());
        for message in messages {
            match message {
                AdnlMessage::CreateChannel { key, date } => {
                    if !self.confirm_channels {
                        // One-shot sessions leave the peer un-channelled on
                        // purpose: this session is dropped right after the
                        // query while the peer keeps the channel.
                        continue;
                    }
                    if self.peer_channel_key == Some(key.0) && !self.channels.is_empty() {
                        // Already negotiated with this peer channel key;
                        // re-negotiating would desynchronise both sides.
                        continue;
                    }
                    self.install_channel(key.0, *date)?;
                    self.peer_channel_key = Some(key.0);
                    self.pending_channel = None;
                    self.send_direct_contents(PacketContents {
                        rand1: vec![0; 7],
                        flags: (),
                        from: None,
                        from_short: None,
                        message: Some(AdnlMessage::ConfirmChannel {
                            key: Int256(self.local_channel.public_key.to_bytes()),
                            peer_key: key.clone(),
                            date: *date,
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
                    .await?;
                }
                AdnlMessage::ConfirmChannel {
                    key,
                    peer_key,
                    date,
                } => {
                    if !self.confirm_channels {
                        continue;
                    }
                    let Some(pending_date) = self.pending_channel else {
                        log::trace!(
                            "ADNL confirmChannel without a pending request ignored: remote_id={}",
                            hex::encode(self.remote_id)
                        );
                        continue;
                    };
                    // `peer_key` must echo the local channel key this session
                    // sent in `createChannel`.  The `date` field is the peer's
                    // own channel key date, which legitimately is older than
                    // our request, so it is not validated here.
                    if peer_key.0 != self.local_channel.public_key.to_bytes() {
                        return Err(AdnlError::ChannelConfirmMismatch {
                            expected: self.local_channel.public_key.to_bytes(),
                            got: peer_key.0,
                        });
                    }
                    if self.peer_channel_key == Some(key.0) && !self.channels.is_empty() {
                        self.pending_channel = None;
                        continue;
                    }
                    log::debug!(
                        "ADNL channel confirmed by peer: remote_id={} date={} requested_date={pending_date}",
                        hex::encode(self.remote_id),
                        date
                    );
                    self.install_channel(key.0, *date)?;
                    self.peer_channel_key = Some(key.0);
                    self.pending_channel = None;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn install_channel(&mut self, remote_channel: [u8; 32], date: i32) -> Result<(), AdnlError> {
        let remote_channel =
            PublicKey::from_bytes(remote_channel).ok_or(AdnlError::InvalidPublicKey)?;
        let shared = self.local_channel.compute_shared_secret(&remote_channel);
        let (outbound, inbound) = ordered_channel_ciphers(self.local_id, self.remote_id, shared);
        let outbound_id = channel_id_for_secret(outbound.secret());
        let inbound_id = channel_id_for_secret(inbound.secret());
        log::debug!(
            "ADNL channel installed: local_id={} remote_id={} out_id={} in_id={} date={date}",
            hex::encode(self.local_id),
            hex::encode(self.remote_id),
            hex::encode(outbound_id),
            hex::encode(inbound_id)
        );
        let channel =
            AdnlChannelPacket::new_directional(outbound_id, inbound_id, outbound, inbound);
        let existing = self
            .channels
            .iter()
            .position(|held| held.inbound_id == inbound_id);
        if let Some(position) = existing {
            // Already negotiated: keep its replay window and only promote it
            // back into the sending slot when the peer switched to it again.
            if position > 0 {
                let channel = self.channels.remove(position);
                self.channels.insert(0, channel);
            }
            return Ok(());
        }
        self.channels.insert(0, channel);
        self.channels.truncate(MAX_SESSION_CHANNELS);
        Ok(())
    }

    /// Builds the `adnl.message.createChannel` this session should send next.
    ///
    /// Used both to initiate a channel and to re-key a channel the peer still
    /// holds from an earlier session of ours: upstream only accepts the new key
    /// when the carried `date` is strictly greater than the date it recorded,
    /// which is why the current time is used here.
    ///
    /// Returns `None` when the session must not negotiate - one-shot sessions
    /// disable it, and a session that already holds a channel has nothing to
    /// ask for - or when the previous attempt is younger than
    /// [`REQUEST_CHANNEL_INTERVAL`].  On `Some`, the peer pair is marked as
    /// awaiting `confirmChannel`, which is what makes the peer's answer
    /// acceptable to [`Self::process_channel_control`].
    fn create_channel_message(&mut self) -> Option<AdnlMessage> {
        if !self.confirm_channels || !self.channels.is_empty() {
            return None;
        }
        if self
            .channel_requested_at
            .is_some_and(|at| at.elapsed() < REQUEST_CHANNEL_INTERVAL)
        {
            return None;
        }
        self.channel_requested_at = Some(std::time::Instant::now());
        let date = now_i32();
        self.pending_channel = Some(date);
        log::debug!(
            "ADNL channel requested: local_id={} remote_id={} key={} date={date}",
            hex::encode(self.local_id),
            hex::encode(self.remote_id),
            hex::encode(self.local_channel.public_key.to_bytes())
        );
        Some(AdnlMessage::CreateChannel {
            key: Int256(self.local_channel.public_key.to_bytes()),
            date,
        })
    }

    async fn request_channel(&mut self) -> Result<(), AdnlError> {
        let Some(message) = self.create_channel_message() else {
            return Ok(());
        };
        self.send_direct_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(message),
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
        .await?;
        Ok(())
    }

    pub async fn establish_channel(&mut self, timeout: Duration) -> Result<(), AdnlError> {
        const MAX_RETRIES: u32 = 3;
        if !self.channels.is_empty() {
            return Ok(());
        }
        let mut attempt = 0u32;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            attempt += 1;
            let date = now_i32();
            self.pending_channel = Some(date);
            self.send_direct_contents(PacketContents {
                rand1: vec![0; 7],
                flags: (),
                from: None,
                from_short: None,
                message: None,
                messages: Some(vec![
                    AdnlMessage::CreateChannel {
                        key: Int256(self.local_channel.public_key.to_bytes()),
                        date,
                    },
                    AdnlMessage::Query {
                        query_id: Int256::random(),
                        query: tl_proto::serialize(DhtMessage::GetSignedAddressList),
                    },
                ]),
                address: None,
                priority_address: None,
                seqno: None,
                confirm_seqno: None,
                recv_addr_list_version: None,
                recv_priority_addr_list_version: None,
                reinit_date: None,
                dst_reinit_date: Some(0),
                signature: None,
                rand2: vec![0; 7],
            })
            .await?;
            let mut last_err = None;
            while self.channels.is_empty() {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(AdnlError::Timeout {
                        operation: "ADNL UDP channel handshake",
                        timeout,
                    });
                }
                match self.recv_timeout(remaining).await {
                    Ok(_) => {}
                    Err(e) => {
                        last_err = Some(e);
                        break;
                    }
                }
            }
            if !self.channels.is_empty() {
                return Ok(());
            }
            if attempt >= MAX_RETRIES {
                return Err(last_err.unwrap_or(AdnlError::Timeout {
                    operation: "ADNL UDP channel handshake",
                    timeout,
                }));
            }
            let backoff = Duration::from_millis(100 * 2u64.pow(attempt - 1));
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::time::sleep(backoff.min(remaining)).await;
        }
    }

    pub async fn send_timeout(
        &mut self,
        contents: PacketContents,
        timeout: Duration,
    ) -> Result<usize, AdnlError> {
        tokio::time::timeout(timeout, self.send_contents(contents))
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP packet send",
                timeout,
            })?
    }

    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<PacketContents, AdnlError> {
        tokio::time::timeout(timeout, self.recv_contents())
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP packet receive",
                timeout,
            })?
    }
}
