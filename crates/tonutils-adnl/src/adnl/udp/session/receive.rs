//! Receive half of a shared session.
//!
//! The session's receive task decodes, validates, and dispatches every
//! datagram the transport's demultiplexer routed to this session, and
//! routes answers of in-flight queries by query id to the calls waiting
//! for them.  Solo sessions have no such task - their consumer drives
//! the receive loop through [`SessionInner::recv_solo_timeout`] - so
//! the validation here serves both transport variants.

use super::*;

/// How long a datagram may wait in a session queue before the wait is
/// logged as its own diagnostic.
///
/// The end-to-end value is what `send_answer` reports; this line only
/// separates a session-queue stall from a demultiplexer or consumer
/// one.
const SLOW_SESSION_WAIT_MS: u128 = 1000;

/// Reads the little-endian TL constructor id of a serialized object.
fn tl_id(data: &[u8]) -> u32 {
    data.get(..4)
        .map_or(0, |id| u32::from_le_bytes([id[0], id[1], id[2], id[3]]))
}

/// What the receive loop must do with one dispatched datagram.
///
/// `PacketContents` is a few hundred bytes; the enum lives on the receive
/// loop's stack only for the duration of one dispatch, so boxing the large
/// variant would add a heap allocation per delivered packet for no benefit.
#[allow(clippy::large_enum_variant)]
enum Dispatch {
    /// Nothing is left for the consumer: the datagram was rejected or
    /// its registered fast answers consumed it.
    Skip,
    /// The decoded packet still needs the consumer inbox.
    Deliver(PacketContents),
    /// Terminal decode failure: end the loop.
    Stop,
}

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
    ///
    /// When the consumer's inbox is full the decoded packet is parked
    /// and the loop keeps reading further datagrams, so their
    /// registered fast answers leave inside the peer's five-second
    /// admission window instead of queueing behind inbox
    /// backpressure; a second consumer-bound packet seen in that state
    /// is dropped with UDP loss semantics and counted as
    /// [`SessionInner::inbox_drops`].
    pub(super) async fn receive_loop(self: Arc<Self>) {
        let mut consecutive_failures: u32 = 0;
        let mut queue = self.queue.lock().await;
        // A decoded packet the consumer's inbox could not take yet.
        // While it waits, later datagrams are still read so their
        // registered answers leave on time.
        let mut parked: Option<PacketContents> = None;
        let mut dropping = false;
        'outer: loop {
            if parked.is_none() {
                let Some((at, datagram)) = queue.recv().await else {
                    break;
                };
                match self
                    .dispatch_datagram(&datagram, at, &mut consecutive_failures)
                    .await
                {
                    Dispatch::Skip => continue,
                    Dispatch::Deliver(contents) => parked = Some(contents),
                    Dispatch::Stop => break,
                }
            }
            let contents = parked.take().expect("the parked packet is set above");
            tokio::select! {
                reserved = self.inbox_tx.reserve() => {
                    let Ok(permit) = reserved else {
                        break 'outer;
                    };
                    permit.send(contents);
                    dropping = false;
                }
                item = queue.recv() => {
                    let Some((at, datagram)) = item else {
                        // The queue closed with a packet still parked:
                        // one last bounded hand-off, then end the loop.
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_millis(200),
                            self.inbox_tx.send(contents),
                        )
                        .await;
                        break 'outer;
                    };
                    parked = Some(contents);
                    match self
                        .dispatch_datagram(&datagram, at, &mut consecutive_failures)
                        .await
                    {
                        Dispatch::Skip => {}
                        Dispatch::Deliver(_dropped) => {
                            // The inbox is still full and one packet is
                            // already parked: the newer consumer-bound
                            // packet is dropped with UDP loss semantics.
                            self.inbox_drops.fetch_add(1, Ordering::Relaxed);
                            if !dropping {
                                dropping = true;
                                log::debug!(
                                    "ADNL session inbox full: dropping consumer-bound packets, remote_id={}",
                                    hex::encode(self.remote_id),
                                );
                            }
                        }
                        Dispatch::Stop => break 'outer,
                    }
                }
            }
        }
    }

    /// Applies the receive-loop work of one queued datagram: the slow
    /// wait log, decode and validation, answer routing, query
    /// bookkeeping, and the registered fast answers.
    async fn dispatch_datagram(
        &self,
        datagram: &[u8],
        at: std::time::Instant,
        consecutive_failures: &mut u32,
    ) -> Dispatch {
        let wait_ms = at.elapsed().as_millis();
        if wait_ms > SLOW_SESSION_WAIT_MS {
            log::debug!(
                "ADNL session queue wait: remote_id={} wait_ms={wait_ms}",
                hex::encode(self.remote_id),
            );
        }
        match self.process_datagram(datagram, consecutive_failures).await {
            Ok(Some(contents)) => {
                if super::super::capture::armed() {
                    super::super::capture::record(
                        "packetContents",
                        Some(*self.remote_addr.lock().unwrap()),
                        &tl_proto::serialize(&contents),
                    );
                    let customs = contents
                        .message
                        .iter()
                        .chain(contents.messages.iter().flatten());
                    for message in customs {
                        if let AdnlMessage::Custom { data } = message {
                            super::super::capture::record("custom", None, data);
                        }
                    }
                }
                self.deliver_answers(&contents);
                self.note_inbound_queries(&contents, at);
                if self.answer_fast_queries(&contents).await {
                    Dispatch::Skip
                } else {
                    Dispatch::Deliver(contents)
                }
            }
            Ok(None) => Dispatch::Skip,
            Err(_) => {
                // Terminal failure: too many consecutive datagrams
                // could not be processed.  The inbox closes and the
                // consumer sees `AdnlError::EndOfStream`.
                log::debug!(
                    "ADNL session receive loop ended: remote_id={} consecutive_failures={consecutive_failures}",
                    hex::encode(self.remote_id),
                );
                Dispatch::Stop
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
    ///
    /// Every delivered answer logs its wall-clock age.  Registration
    /// happens just before the query goes on the wire, so the age is
    /// what the caller would have measured end to end.  An answer whose
    /// waiter is already gone (the caller timed out, or the send
    /// failed) logs the same age from the retained registration
    /// instead - the only way to see that a peer answered, just too
    /// late.
    fn deliver_answers(&self, contents: &PacketContents) {
        let mut pending = self.pending.lock().unwrap();
        let recent = self.recent.lock().unwrap();
        if pending.is_empty() && recent.is_empty() {
            return;
        }
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Answer { query_id, answer } = message {
                if let Some((registered, waiter)) = pending.remove(&query_id.0) {
                    log::debug!(
                        "ADNL answer age: peer={:?} query_id={} age_ms={}",
                        self.remote_id,
                        query_id.to_hex(),
                        registered.elapsed().as_millis(),
                    );
                    let _ = waiter.send(answer.clone());
                } else if let Some((_, registered)) =
                    recent.iter().find(|(id, _)| id == &query_id.0)
                {
                    log::debug!(
                        "ADNL late answer: peer={:?} query_id={} age_ms={} waiter gone",
                        self.remote_id,
                        query_id.to_hex(),
                        registered.elapsed().as_millis(),
                    );
                }
            }
        }
    }

    /// Answers one decoded packet from a registered fast answer, when
    /// the packet is a single query whose outer constructor has one.
    ///
    /// Returns `true` when the answer left and the packet must not reach
    /// the consumer: the registration speaks for the query.  A packet
    /// carrying anything else - control messages, answers, custom
    /// payloads, or a query the closure declines - is left to the
    /// ordinary path untouched.
    async fn answer_fast_queries(&self, contents: &PacketContents) -> bool {
        if contents
            .messages
            .as_ref()
            .is_some_and(|messages| !messages.is_empty())
        {
            return false;
        }
        let Some(AdnlMessage::Query { query_id, query }) = &contents.message else {
            return false;
        };
        let registered = {
            let answers = self.fast_answers.lock().unwrap();
            answers.get(&tl_id(query)).cloned()
        };
        let Some(answer) = registered else {
            return false;
        };
        let Some(payload) = answer(query) else {
            return false;
        };
        if let Err(error) = self.send_answer_to(query_id, payload).await {
            log::trace!(
                "ADNL fast answer send failed: peer={:?} error={error}",
                self.remote_id,
            );
        }
        true
    }

    /// Remembers the queries one accepted packet carries, with the
    /// instant its datagram was read off the socket.
    ///
    /// [`Self::log_answer_latency`] reads the instant back when the
    /// consumer answers, so its log line spans the whole in-process
    /// leg: the demultiplexer channel, the session queue, and the
    /// consumer's own turn before it sends.
    fn note_inbound_queries(&self, contents: &PacketContents, at: std::time::Instant) {
        let mut inbound = self.inbound_queries.lock().unwrap();
        for message in contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten())
        {
            if let AdnlMessage::Query { query_id, query } = message {
                let kind = query
                    .get(..4)
                    .and_then(|id| id.try_into().ok())
                    .map_or(0, u32::from_le_bytes);
                inbound.push_back((query_id.0, at, kind));
            }
        }
        while inbound.len() > MAX_INBOUND_QUERIES {
            inbound.pop_front();
        }
    }

    /// Logs the in-process wait of one answered query and drops its
    /// record.
    ///
    /// The peer measures its `overlay.ping` round trip against a
    /// deadline before marking this node alive and admitting it into
    /// its broadcast fan-out (upstream `OverlayImpl::check_pending_peer`
    /// allows five seconds), so this value is what decides whether
    /// pushes ever arrive.  It is stamped at the socket read, not at
    /// the session's turn, so a stalled demultiplexer or session task
    /// shows up in full.
    pub(super) fn log_answer_latency(&self, query_id: &Int256) {
        let at = {
            let mut inbound = self.inbound_queries.lock().unwrap();
            inbound
                .iter()
                .position(|(id, _, _)| id == &query_id.0)
                .and_then(|index| inbound.remove(index))
                .map(|(_, at, kind)| (at, kind))
        };
        let Some((at, kind)) = at else {
            return;
        };
        log::debug!(
            "ADNL answer latency: peer={:?} query_id={} kind={kind:08x} latency_ms={}",
            self.remote_id,
            query_id.to_hex(),
            at.elapsed().as_millis(),
        );
    }

    /// Registers `query_id` for answer routing and returns the receiver.
    ///
    /// Must be called before the query goes on the wire, otherwise an
    /// immediate answer can arrive before there is a waiter for it.
    /// The registration instant is stored with the waiter so a
    /// delivered answer can log its real age.
    pub(super) fn register_pending(
        &self,
        query_id: Int256,
    ) -> tokio::sync::oneshot::Receiver<Vec<u8>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(query_id.0, (std::time::Instant::now(), tx));
        rx
    }

    /// Drops the registration of a query whose answer never came.
    ///
    /// The registration instant moves to the bounded late-answer
    /// tracker instead of being lost, so an answer that arrives after
    /// the caller gave up still logs how late it was.
    pub(super) fn unregister_pending(&self, query_id: &[u8; 32]) {
        let registered = self
            .pending
            .lock()
            .unwrap()
            .remove(query_id)
            .map(|(registered, _)| registered);
        let Some(registered) = registered else {
            return;
        };
        let mut recent = self.recent.lock().unwrap();
        if recent.len() >= MAX_RECENT_QUERIES {
            recent.pop_front();
        }
        recent.push_back((*query_id, registered));
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
            let at = std::time::Instant::now();
            if size > MAX_UDP_PACKET_SIZE {
                return Err(AdnlError::TooLongPacket);
            }
            if let Some(contents) = self
                .process_datagram(&packet[..size], &mut consecutive_failures)
                .await?
            {
                self.deliver_answers(&contents);
                self.note_inbound_queries(&contents, at);
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
