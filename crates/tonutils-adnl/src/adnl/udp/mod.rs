//! Datagram framing for ADNL sessions.
//!
//! UDP is deliberately exposed as a datagram primitive.  Handshake and peer
//! discovery remain owned by the caller because a UDP endpoint can be shared
//! by several ADNL peers and the protocol does not provide stream semantics.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};

use tokio_util::bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tonutils_tl::Message as AdnlMessage;

use crate::{AdnlAesParams, AdnlCodec, AdnlError};

mod cipher;
mod session;

pub use cipher::{
    AdnlChannelCipher, AdnlChannelPacket, channel_id_for_secret, decrypt_direct, encrypt_direct,
    ordered_channel_ciphers, reverse_channel_secret,
};
pub use session::AdnlUdpSession;

/// Maximum encoded ADNL datagram accepted by the native UDP helper.
pub const MAX_UDP_PACKET_SIZE: usize = 64 * 1024;

/// Highest number of negotiated channels a session keeps for one peer.
///
/// Index 0 is the channel used for sending; the remainder are previous
/// channels that are still decoded so packets in flight during a re-key are
/// not dropped as unknown prefixes.  Upstream keeps the same set in
/// `AdnlPeerPairImpl::channels_`.
const MAX_SESSION_CHANNELS: usize = 3;

/// Minimum distance between two `adnl.message.createChannel` attempts of one
/// session, so an unanswerable peer cannot turn re-keying into a packet flood.
const REQUEST_CHANNEL_INTERVAL: Duration = Duration::from_secs(10);

/// Query ids one socket keeps for the stray-answer diagnostic.
///
/// A live session sends one `overlay.getRandomPeers` per keepalive interval, so
/// this window covers many multiples of that interval; a lookup socket sends
/// far fewer.  Ids fall off the front, which can only turn a diagnostic line
/// into silence, never into a wrong answer.
const MAX_TRACKED_QUERIES: usize = 128;

/// Current Unix timestamp clamped to i32::MAX.
///
/// The ADNL wire protocol uses i32 for dates, which overflows after 2038-01-19
/// 03:14:07 UTC. This helper clamps the value so callers never panic on
/// conversion. Callers that need to store or send dates beyond 2038 should use
/// a wider type internally and only convert at the wire boundary.
pub fn now_i32() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    secs.min(i32::MAX as u64) as i32
}

/// Process-wide reinit date advertised in outgoing direct packets.
///
/// Mirrors upstream `Adnl::adnl_start_time()` (`adnl/adnl-peer-table.cpp`):
/// the value is the Unix time captured on first use and stays constant for
/// the lifetime of the process.  It is written into every direct
/// `adnl.packetContents` as `reinit_date`, with `dst_reinit_date` carrying the
/// peer's own announced date (see [`AdnlUdpSession::send_direct_contents`]).
///
/// A per-packet value (for example `now_i32()`) must never be used here:
/// upstream treats a strictly increasing `reinit_date` as a peer restart and
/// calls `AdnlPeerPairImpl::reinit()` on every datagram, which resets the
/// peer's sequence numbers and unregisters its ADNL channel.  That makes the
/// peer drop subsequent packets with "new ack seqno", so the channel never
/// becomes ready and no answers or broadcasts are ever delivered.
pub fn local_reinit_date() -> i32 {
    static REINIT_DATE: OnceLock<i32> = OnceLock::new();
    *REINIT_DATE.get_or_init(now_i32)
}

/// ADNL state that belongs to a peer pair rather than to one socket.
///
/// Upstream keeps a single `AdnlPeerPairImpl` per (local node id, peer node
/// id) pair, so `out_seqno_`, the acknowledged `in_seqno_` and the peer's
/// announced `reinit_date` survive socket reconnects.  This crate opens a
/// fresh [`AdnlUdpSession`] per lookup/bootstrap phase instead, and the peer
/// keeps validating the *same* pair state for every one of them.  A second
/// session that restarts `seqno` at 1 therefore has all of its packets
/// rejected as replays by the peer, which shows up as unanswered queries and
/// `overlay_packets: 0`.
///
/// The counters that the peer validates across the whole pair are kept here
/// keyed by the remote ADNL id.  Replay-window bookkeeping (`received`) stays
/// per session: it only guards against duplicates inside one session, and
/// their sequence numbers are monotonic anyway.
#[derive(Default)]
struct PeerPairState {
    /// Outgoing sequence number; shared by direct and channel packets.
    next_seqno: u64,
    /// Highest sequence number received from the peer.
    highest_seqno: u64,
    /// Latest `reinit_date` announced by the peer, `0` when none was seen yet.
    reinit_date: i32,
}

/// Returns a short name for an `adnl.Message` used in receive traces.
fn message_kind(message: Option<&AdnlMessage>) -> &'static str {
    match message {
        None => "none",
        Some(AdnlMessage::CreateChannel { .. }) => "createChannel",
        Some(AdnlMessage::ConfirmChannel { .. }) => "confirmChannel",
        Some(AdnlMessage::Custom { .. }) => "custom",
        Some(AdnlMessage::Nop) => "nop",
        Some(AdnlMessage::Reinit { .. }) => "reinit",
        Some(AdnlMessage::Query { .. }) => "query",
        Some(AdnlMessage::Answer { .. }) => "answer",
        Some(_) => "other",
    }
}

/// Renders the message vector of a packet for receive traces.
fn message_vector(messages: &[AdnlMessage]) -> String {
    messages
        .iter()
        .map(|message| message_kind(Some(message)))
        .collect::<Vec<_>>()
        .join("+")
}

fn peer_pair_states() -> &'static Mutex<HashMap<[u8; 32], PeerPairState>> {
    static STATES: OnceLock<Mutex<HashMap<[u8; 32], PeerPairState>>> = OnceLock::new();
    STATES.get_or_init(Default::default)
}

fn with_peer_pair_state<R>(remote_id: &[u8; 32], f: impl FnOnce(&mut PeerPairState) -> R) -> R {
    let mut states = peer_pair_states()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(states.entry(*remote_id).or_default())
}

/// Allocates the next outgoing sequence number for `remote_id`.
fn next_outgoing_seqno(remote_id: &[u8; 32]) -> u64 {
    with_peer_pair_state(remote_id, |state| {
        state.next_seqno = state.next_seqno.saturating_add(1);
        state.next_seqno
    })
}

/// Current outgoing sequence number of `remote_id` without allocating one.
fn outgoing_seqno(remote_id: &[u8; 32]) -> u64 {
    with_peer_pair_state(remote_id, |state| state.next_seqno)
}

/// Highest sequence number received from `remote_id`.
fn highest_received_seqno(remote_id: &[u8; 32]) -> u64 {
    with_peer_pair_state(remote_id, |state| state.highest_seqno)
}

/// Records a received sequence number and returns the new high-water mark.
fn record_received_seqno(remote_id: &[u8; 32], seqno: u64) -> u64 {
    with_peer_pair_state(remote_id, |state| {
        state.highest_seqno = state.highest_seqno.max(seqno);
        state.highest_seqno
    })
}

/// Latest `reinit_date` announced by `remote_id`.
fn peer_reinit_date(remote_id: &[u8; 32]) -> i32 {
    with_peer_pair_state(remote_id, |state| state.reinit_date)
}

/// Returns whether `date` is a *new* peer epoch, applying the reset when it is.
///
/// Mirrors `AdnlPeerPairImpl::reinit`: a strictly later `reinit_date` means
/// the peer restarted, so both directions of the pair's sequence numbers are
/// cleared.  Returns `true` when the caller must also drop its session-local
/// state.
fn note_peer_reinit_date(remote_id: &[u8; 32], date: i32) -> bool {
    with_peer_pair_state(remote_id, |state| {
        if state.reinit_date == 0 {
            state.reinit_date = date;
            false
        } else if state.reinit_date < date {
            state.reinit_date = date;
            state.next_seqno = 0;
            state.highest_seqno = 0;
            true
        } else {
            false
        }
    })
}

/// An encrypted ADNL datagram peer bound to one remote endpoint.
pub struct AdnlUdpPeer {
    remote: SocketAddr,
    codec: AdnlCodec,
    seen: VecDeque<[u8; 32]>,
}

impl AdnlUdpPeer {
    /// Creates a UDP peer using client-side session keys.
    pub fn client(remote: SocketAddr, params: &AdnlAesParams) -> Self {
        Self {
            remote,
            codec: AdnlCodec::client(params),
            seen: VecDeque::new(),
        }
    }

    /// Creates a UDP peer using server-side session keys.
    pub fn server(remote: SocketAddr, params: &AdnlAesParams) -> Self {
        Self {
            remote,
            codec: AdnlCodec::server(params),
            seen: VecDeque::new(),
        }
    }

    pub fn remote(&self) -> SocketAddr {
        self.remote
    }

    /// Encodes one payload as exactly one UDP datagram.
    pub fn encode(&mut self, payload: Bytes) -> Result<Bytes, AdnlError> {
        let mut output = BytesMut::new();
        self.codec.encode(payload, &mut output)?;
        if output.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        Ok(output.freeze())
    }

    /// Decodes one complete UDP datagram and rejects trailing frames.
    pub fn decode(&mut self, datagram: &[u8]) -> Result<Bytes, AdnlError> {
        if datagram.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        let packet_hash: [u8; 32] = Sha256::digest(datagram).into();
        if self.seen.contains(&packet_hash) {
            return Err(AdnlError::ReplayDetected);
        }
        let mut input = BytesMut::from(datagram);
        let payload = self
            .codec
            .decode(&mut input)?
            .ok_or(AdnlError::EndOfStream)?;
        if !input.is_empty() {
            return Err(AdnlError::TooLongPacket);
        }
        self.seen.push_back(packet_hash);
        if self.seen.len() > 4096 {
            self.seen.pop_front();
        }
        Ok(payload)
    }
}

/// Tokio UDP endpoint for one authenticated ADNL datagram session.
pub struct AdnlUdpSocket {
    socket: tokio::net::UdpSocket,
    peer: AdnlUdpPeer,
}

impl AdnlUdpSocket {
    pub async fn bind(
        local: SocketAddr,
        remote: SocketAddr,
        params: &AdnlAesParams,
    ) -> Result<Self, AdnlError> {
        let socket = tokio::net::UdpSocket::bind(local).await?;
        socket.connect(remote).await?;
        Ok(Self {
            socket,
            peer: AdnlUdpPeer::client(remote, params),
        })
    }

    pub fn remote(&self) -> SocketAddr {
        self.peer.remote()
    }

    pub async fn send(&mut self, payload: Bytes) -> Result<usize, AdnlError> {
        let packet = self.peer.encode(payload)?;
        Ok(self.socket.send(&packet).await?)
    }

    pub async fn send_timeout(
        &mut self,
        payload: Bytes,
        timeout: Duration,
    ) -> Result<usize, AdnlError> {
        tokio::time::timeout(timeout, self.send(payload))
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP send",
                timeout,
            })?
    }

    pub async fn recv(&mut self) -> Result<Bytes, AdnlError> {
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        let size = self.socket.recv(&mut packet).await?;
        if size > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        self.peer.decode(&packet[..size])
    }

    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<Bytes, AdnlError> {
        tokio::time::timeout(timeout, self.recv())
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP receive",
                timeout,
            })?
    }
}

#[cfg(test)]
mod tests;
