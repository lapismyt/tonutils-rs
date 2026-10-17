//! Receive half of a shared session.
//!
//! The session's receive task decodes, validates, and dispatches every
//! datagram the transport's demultiplexer routed to this session, and
//! routes answers of in-flight queries by query id to the calls waiting
//! for them.  Solo sessions have no such task - their consumer drives
//! the receive loop through [`SessionInner::recv_solo_timeout`] - so
//! the validation here serves both transport variants.

use super::*;

impl SessionInner {
    /// Decodes, validates, and dispatches every datagram routed
    /// to a shared session.
    ///
    /// This is the receive half that [`AdnlUdpSession::recv_contents`]
    /// used to inline: the per-peer checks of upstream
    /// `AdnlPeerPairImpl::receive_packet_checked`, channel control, and
    /// the diagnostics all run here, under the session state lock, and
    /// the decoded packet is then handed to the consumer.  Answers to
    /// in-flight queries are routed by query id before the packet
    /// reaches the consumer, so a shared session can serve concurrent
    /// callers.
    pub(super) async fn receive_loop(self: Arc<Self>) {
        let mut consecutive_failures: u32 = 0;
        let mut queue = self.queue.lock().await;
        while let Some(datagram) = queue.recv().await {
            let processed = self
                .process_datagram(&datagram, &mut consecutive_failures)
                .await;
            match processed {
                Ok(Some(contents)) => {
                    self.deliver_answers(&contents);
                    if self.inbox_tx.send(contents).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    // Terminal failure: too many consecutive datagrams
                    // could not be processed.  The inbox closes and the
                    // consumer sees `AdnlError::EndOfStream`.
                    log::debug!(
                        "ADNL session receive loop ended: remote_id={} consecutive_failures={consecutive_failures}",
                        hex::encode(self.remote_id),
                    );
                    break;
                }
            }
        }
    }

    /// Runs the decode and validation of one datagram.
    ///
    /// Returns `Ok(None)` for rejected datagrams, `Ok(Some)` for
    /// accepted ones, and `Err` when the session hit its consecutive
    /// failure limit.
    async fn process_datagram(
        &self,
        datagram: &[u8],
        consecutive_failures: &mut u32,
    ) -> Result<Option<PacketContents>, AdnlError> {
        let mut state = self.state.lock().await;
        let Some(contents) = self
            .decode_packet(&mut state, datagram, consecutive_failures)
            .await?
        else {
            return Ok(None);
        };
        // Per-peer checks shared by direct and channel packets,
        // mirroring upstream `AdnlPeerPairImpl::receive_packet_checked`.
        // ADNL keeps one sequence number space per peer pair, so both
        // transports are validated together here.
        if !self.apply_peer_reinit(&mut state, contents.reinit_date) {
            return Ok(None);
        }
        if let Some(seqno) = contents.seqno {
            let highest = highest_received_seqno(&self.remote_id);
            if seqno == 0
                || state.received.contains(&seqno)
                || (highest > 4096 && seqno + 4096 < highest)
            {
                log::trace!(
                    "recv_contents: seqno dropped (seqno={seqno}, highest={highest}, received_len={})",
                    state.received.len(),
                );
                Self::count_failure(consecutive_failures, 256)?;
                return Ok(None);
            }
            record_received_seqno(&self.remote_id, seqno);
            state.received.push_back(seqno);
            while state.received.len() > 4096 {
                state.received.pop_front();
            }
        }
        if let Some(address) = &contents.address
            && state
                .peer_addr_list_version
                .is_none_or(|current| address.version > current)
        {
            state.peer_addr_list_version = Some(address.version);
        }
        if self
            .process_channel_control(&mut state, &contents)
            .await
            .is_err()
        {
            Self::count_failure(consecutive_failures, 256)?;
            return Ok(None);
        }
        self.log_stray_answers(&state, &contents);
        self.log_recv_diagnostics(&contents);
        log::trace!(
            "recv: from={} seqno={:?} confirm={:?} reinit={:?} kind={} messages={}",
            self.remote_addr.lock().unwrap(),
            contents.seqno,
            contents.confirm_seqno,
            contents.reinit_date,
            message_kind(contents.message.as_ref()),
            contents
                .messages
                .as_deref()
                .map_or_else(|| "-".into(), message_vector),
        );
        Ok(Some(contents))
    }

    /// Routes answers of in-flight queries to the calls waiting for them.
    fn deliver_answers(&self, contents: &PacketContents) {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            return;
        }
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Answer { query_id, answer } = message
                && let Some(waiter) = pending.remove(&query_id.0)
            {
                let _ = waiter.send(answer.clone());
            }
        }
    }

    /// Registers `query_id` for answer routing and returns the receiver.
    ///
    /// Must be called before the query goes on the wire, otherwise an
    /// immediate answer can arrive before there is a waiter for it.
    pub(super) fn register_pending(
        &self,
        query_id: Int256,
    ) -> tokio::sync::oneshot::Receiver<Vec<u8>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(query_id.0, tx);
        rx
    }

    /// Drops the registration of a query whose answer never came.
    pub(super) fn unregister_pending(&self, query_id: &[u8; 32]) {
        self.pending.lock().unwrap().remove(query_id);
    }

    /// Reads, decodes, and returns the next packet of a solo
    /// session, waiting at most `timeout`.
    ///
    /// `Ok(None)` means the wait ran out without a valid packet.
    /// Answers to in-flight queries are routed to their waiters
    /// as a side effect of decoding, exactly as in
    /// [`AdnlUdpSession::recv_contents`].
    pub(super) async fn recv_solo_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<PacketContents>, AdnlError> {
        let Source::Solo(socket) = &self.source else {
            return Ok(None);
        };
        let deadline = tokio::time::Instant::now() + timeout;
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        let mut consecutive_failures: u32 = 0;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let size = match tokio::time::timeout(remaining, socket.recv(&mut packet)).await {
                Ok(Ok(size)) => size,
                Ok(Err(error)) => return Err(AdnlError::from(error)),
                Err(_) => return Ok(None),
            };
            if size > MAX_UDP_PACKET_SIZE {
                return Err(AdnlError::TooLongPacket);
            }
            if let Some(contents) = self
                .process_datagram(&packet[..size], &mut consecutive_failures)
                .await?
            {
                self.deliver_answers(&contents);
                return Ok(Some(contents));
            }
        }
    }

    /// Awaits the answer of a pending query, cleaning up on every exit
    /// path.
    ///
    /// A shared session's receive task routes the answer as
    /// soon as it arrives, so waiting here is enough.  A solo
    /// session has no receive task, so this call keeps its
    /// socket driven while it waits: every datagram received
    /// is processed by [`Self::recv_solo_timeout`], which
    /// routes the answer - if the datagram carries it - to
    /// this call's waiter.
    pub(super) async fn await_pending_answer(
        &self,
        query_id: Int256,
        timeout: Duration,
        operation: &'static str,
        mut rx: tokio::sync::oneshot::Receiver<Vec<u8>>,
    ) -> Result<Vec<u8>, AdnlError> {
        if matches!(self.source, Source::Solo(_)) {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                match rx.try_recv() {
                    Ok(answer) => {
                        self.unregister_pending(&query_id.0);
                        return Ok(answer);
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.unregister_pending(&query_id.0);
                        return Err(AdnlError::EndOfStream);
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    self.unregister_pending(&query_id.0);
                    return Err(AdnlError::Timeout { operation, timeout });
                }
                if let Err(error) = self.recv_solo_timeout(remaining).await {
                    self.unregister_pending(&query_id.0);
                    return Err(error);
                }
            }
        }
        let waited = tokio::time::timeout(timeout, rx).await;
        self.unregister_pending(&query_id.0);
        match waited {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err(AdnlError::EndOfStream),
            Err(_) => Err(AdnlError::Timeout { operation, timeout }),
        }
    }

    /// Applies upstream's peer-reinit rules to an accepted packet.
    ///
    /// Mirrors `AdnlPeerPairImpl::receive_packet_checked` and
    /// `AdnlPeerPairImpl::reinit` in `adnl/adnl-peer.cpp`: the first
    /// announced `reinit_date` is only recorded, a *later* one resets
    /// the per-peer state (sequence numbers and the ADNL channel), and a
    /// packet carrying an older date is dropped as stale.
    ///
    /// Returns `false` when the packet must be ignored.
    fn apply_peer_reinit(&self, state: &mut SessionState, reinit_date: Option<i32>) -> bool {
        let date = reinit_date.unwrap_or(0);
        if note_peer_reinit_date(&self.remote_id, date) {
            Self::reset_peer_state(state);
            log::debug!(
                "ADNL peer reinitialized: remote_id={} reinit_date={date}",
                hex::encode(self.remote_id),
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

    /// Drops state that only belongs to the peer's previous epoch,
    /// mirroring `AdnlPeerPairImpl::reinit`.
    fn reset_peer_state(state: &mut SessionState) {
        state.received.clear();
        state.channels.clear();
        state.pending_channel = None;
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
    /// Returns `None` when the datagram was rejected.  Rejections are
    /// counted through `consecutive_failures`, and hitting the limit
    /// aborts the session with [`AdnlError::IntegrityError`].
    ///
    /// Sequence numbers are deliberately *not* validated here: they are
    /// validated once in [`SessionInner::process_datagram`] together
    /// with the peer's `reinit_date`, exactly like upstream does after a
    /// packet has been decrypted, because direct and channel packets
    /// share the same sequence number space.  The channel still performs
    /// its own replay guard for standalone use, seeded from the shared
    /// peer pair counters below.
    async fn decode_packet(
        &self,
        state: &mut SessionState,
        packet: &[u8],
        consecutive_failures: &mut u32,
    ) -> Result<Option<PacketContents>, AdnlError> {
        const MAX_CONSECUTIVE_FAILURES: u32 = 256;
        let size = packet.len();
        let channel_index = state.channels.iter().position(|channel| {
            size >= channel.inbound_id.len()
                && packet[..channel.inbound_id.len()] == channel.inbound_id
        });
        if let Some(index) = channel_index {
            let channel = &mut state.channels[index];
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
            let channel_ids = if state.channels.is_empty() {
                String::new()
            } else {
                format!(
                    " in_id={} out_id={}",
                    hex::encode(state.channels[0].inbound_id),
                    hex::encode(state.channels[0].outbound_id)
                )
            };
            if self.confirm_channels.load(Ordering::Relaxed) && state.channels.is_empty() {
                // The prefix is neither ours nor any channel this session
                // holds, so the peer is still using a channel negotiated by an
                // earlier session of ours - our local channel key changed with
                // it, so it can never be decrypted here.  Ask the peer to
                // re-key; `request_channel` rate limits itself to one attempt
                // per ten seconds.
                if let Err(error) = self.request_channel(state).await {
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
        if let Some(flags) = raw_packet_flags(&payload) {
            log::debug!(
                "flags probe: kind=direct raw=0x{flags:08x} recv_v={:?} recv_prio={:?}",
                contents.recv_addr_list_version,
                contents.recv_priority_addr_list_version,
            );
        }
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
}
