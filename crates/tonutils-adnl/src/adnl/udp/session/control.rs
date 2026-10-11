//! ADNL channel control.
//!
//! The `adnl.message.createChannel` / `confirmChannel` handshake and the
//! packet dispatch that reacts to it.  Installing a channel registers its
//! id with the shared transport, so channel packets keep reaching this
//! session even after the peer changes its source address.

use super::*;

impl SessionInner {
    /// Processes channel control messages of an accepted packet.
    ///
    /// `createChannel` installs the channel and replies with
    /// `confirmChannel`; `confirmChannel` completes a handshake this
    /// session started.  One-shot sessions ignore both, leaving the peer
    /// un-channelled on purpose.
    pub(super) async fn process_channel_control(
        &self,
        state: &mut SessionState,
        contents: &PacketContents,
    ) -> Result<(), AdnlError> {
        let messages = contents
            .message
            .iter()
            .chain(contents.messages.iter().flatten());
        for message in messages {
            match message {
                AdnlMessage::CreateChannel { key, date } => {
                    if !self.confirm_channels.load(Ordering::Relaxed) {
                        // One-shot sessions leave the peer un-channelled on
                        // purpose: this session is dropped right after the
                        // query while the peer keeps the channel.
                        continue;
                    }
                    if state.peer_channel_key == Some(key.0) && !state.channels.is_empty() {
                        // Already negotiated with this peer channel key;
                        // re-negotiating would desynchronise both sides.
                        continue;
                    }
                    self.install_channel(state, key.0, *date)?;
                    state.peer_channel_key = Some(key.0);
                    state.pending_channel = None;
                    self.send_direct_locked(
                        state,
                        PacketContents {
                            rand1: vec![0; 7],
                            flags: (),
                            from: None,
                            from_short: None,
                            message: Some(AdnlMessage::ConfirmChannel {
                                key: Int256(state.local_channel.public_key.to_bytes()),
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
                        },
                    )
                    .await?;
                }
                AdnlMessage::ConfirmChannel {
                    key,
                    peer_key,
                    date,
                } => {
                    if !self.confirm_channels.load(Ordering::Relaxed) {
                        continue;
                    }
                    let Some(pending_date) = state.pending_channel else {
                        log::trace!(
                            "ADNL confirmChannel without a pending request ignored: remote_id={}",
                            hex::encode(self.remote_id),
                        );
                        continue;
                    };
                    // `peer_key` must echo the local channel key this
                    // session sent in `createChannel`.  The `date` field
                    // is the peer's own channel key date, which
                    // legitimately is older than our request, so it is
                    // not validated here.
                    if peer_key.0 != state.local_channel.public_key.to_bytes() {
                        return Err(AdnlError::ChannelConfirmMismatch {
                            expected: state.local_channel.public_key.to_bytes(),
                            got: peer_key.0,
                        });
                    }
                    if state.peer_channel_key == Some(key.0) && !state.channels.is_empty() {
                        state.pending_channel = None;
                        continue;
                    }
                    log::debug!(
                        "ADNL channel confirmed by peer: remote_id={} date={} requested_date={pending_date}",
                        hex::encode(self.remote_id),
                        date
                    );
                    self.install_channel(state, key.0, *date)?;
                    state.peer_channel_key = Some(key.0);
                    state.pending_channel = None;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn install_channel(
        &self,
        state: &mut SessionState,
        remote_channel: [u8; 32],
        date: i32,
    ) -> Result<(), AdnlError> {
        let remote_channel =
            PublicKey::from_bytes(remote_channel).ok_or(AdnlError::InvalidPublicKey)?;
        let shared = state.local_channel.compute_shared_secret(&remote_channel);
        let (outbound, inbound) = ordered_channel_ciphers(self.local_id, self.remote_id, shared);
        let outbound_id = channel_id_for_secret(outbound.secret());
        let inbound_id = channel_id_for_secret(inbound.secret());
        log::debug!(
            "ADNL channel installed: local_id={} remote_id={} out_id={} in_id={} date={date}",
            hex::encode(self.local_id),
            hex::encode(self.remote_id),
            hex::encode(outbound_id),
            hex::encode(inbound_id),
        );
        let channel =
            AdnlChannelPacket::new_directional(outbound_id, inbound_id, outbound, inbound);
        let existing = state
            .channels
            .iter()
            .position(|held| held.inbound_id == inbound_id);
        if let Some(position) = existing {
            // Already negotiated: keep its replay window and only promote
            // it back into the sending slot when the peer switched to it
            // again.
            if position > 0 {
                let channel = state.channels.remove(position);
                state.channels.insert(0, channel);
            }
            return Ok(());
        }
        state.channels.insert(0, channel);
        state.channels.truncate(MAX_SESSION_CHANNELS);
        self.register_channel_route(inbound_id);
        Ok(())
    }

    /// Builds the `adnl.message.createChannel` this session should send
    /// next.
    ///
    /// Used both to initiate a channel and to re-key a channel the peer
    /// still holds from an earlier session of ours: upstream only accepts
    /// the new key when the carried `date` is strictly greater than the
    /// date it recorded, which is why the current time is used here.
    ///
    /// Returns `None` when the session must not negotiate - one-shot
    /// sessions disable it, and a session that already holds a channel
    /// has nothing to ask for - or when the previous attempt is younger
    /// than [`REQUEST_CHANNEL_INTERVAL`].  On `Some`, the session is
    /// marked as awaiting `confirmChannel`, which is what makes the
    /// peer's answer acceptable to [`Self::process_channel_control`].
    pub(super) fn create_channel_message(&self, state: &mut SessionState) -> Option<AdnlMessage> {
        if !self.confirm_channels.load(Ordering::Relaxed) || !state.channels.is_empty() {
            return None;
        }
        if state
            .channel_requested_at
            .is_some_and(|at| at.elapsed() < REQUEST_CHANNEL_INTERVAL)
        {
            return None;
        }
        state.channel_requested_at = Some(std::time::Instant::now());
        let date = now_i32();
        state.pending_channel = Some(date);
        log::debug!(
            "ADNL channel requested: local_id={} remote_id={} key={} date={date}",
            hex::encode(self.local_id),
            hex::encode(self.remote_id),
            hex::encode(state.local_channel.public_key.to_bytes()),
        );
        Some(AdnlMessage::CreateChannel {
            key: Int256(state.local_channel.public_key.to_bytes()),
            date,
        })
    }

    /// Sends `createChannel` when a session is allowed and rate limited
    /// to do so.
    pub(super) async fn request_channel(&self, state: &mut SessionState) -> Result<(), AdnlError> {
        let Some(message) = self.create_channel_message(state) else {
            return Ok(());
        };
        self.send_direct_locked(
            state,
            PacketContents {
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
            },
        )
        .await?;
        Ok(())
    }
}
