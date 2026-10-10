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
mod transport;

pub use cipher::{
    AdnlChannelCipher, AdnlChannelPacket, channel_id_for_secret, decrypt_direct, encrypt_direct,
    ordered_channel_ciphers, reverse_channel_secret,
};
pub use session::AdnlUdpSession;
pub use transport::AdnlUdpTransport;

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
    /// Highest `adnl.addressList.version` this process stamped for `remote_id`.
    ///
    /// The peer keeps exactly one source address per node id and replaces it
    /// only on a strictly greater version, so comparing this value against the
    /// `recv_addr_list_version` the peer echoes back tells whether the address
    /// the peer will use is one this process stamped or an older one.
    our_addr_version: i32,
    /// Local port of the socket that stamped `our_addr_version`.
    ///
    /// One node id sends from one socket, so this is the port the peer
    /// addresses this node on whenever the recorded version wins.
    our_addr_socket: u16,
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

/// Records the `adnl.addressList.version` this process just stamped for
/// `remote_id` from `socket`.
///
/// Upstream echoes the version it currently holds for us back in every packet
/// (`AdnlPeerPairImpl::send_packet` writes `addr_list_.version()` into
/// `recv_addr_list_version`), so keeping our own last stamp here is what lets
/// a receive-side diagnostic tell "the peer moved on to a newer address than
/// the one we sent" from "the peer still uses the address we sent last".
///
/// Only a *strictly* greater version replaces the owner, mirroring upstream's
/// `AdnlPeerPairImpl::update_addr_list`: within the same second the incumbent
/// socket keeps the address, so the recorded port stays the one the peer is
/// actually using.
fn note_our_addr_version(remote_id: &[u8; 32], version: i32, socket: u16) {
    with_peer_pair_state(remote_id, |state| {
        if version > state.our_addr_version {
            state.our_addr_version = version;
            state.our_addr_socket = socket;
        }
    })
}

/// Version this process last stamped for `remote_id` and the local
/// port of the socket that stamped it.
fn our_addr_view(remote_id: &[u8; 32]) -> (i32, u16) {
    with_peer_pair_state(remote_id, |state| {
        (state.our_addr_version, state.our_addr_socket)
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

/// Reads the `flags:#` word straight out of a serialized `adnl.packetContents`.
///
/// [`PacketContents`] keeps its own `flags` field as `()`, so a decoded value
/// cannot tell a bit that was genuinely absent from the wire apart from one the
/// field mapping failed to surface.  For `recv_addr_list_version` those two
/// readings are opposites: absent means the peer holds no address for this node
/// and therefore cannot originate an `overlay.ping`, while present-but-ignored
/// would point at this crate's TL mapping instead.  Upstream always answers from
/// a connection it holds an address on
/// (`AdnlPeerPairImpl::get_conn` returns `no active connections` otherwise), so
/// which of the two it is decides whether the missing address is real.
///
/// Layout is `ctor:u32 = 0xd142cd89`, then `rand1:bytes` as a length prefix
/// plus data, then `flags:u32`.  Upstream fills `rand1` with exactly 7 or 15
/// random bytes (`AdnlPacket::init_random`), which is what makes the offset
/// findable, and the length prefix is read from the first byte only - that
/// value is identical whether the prefix is one byte or a little-endian
/// `u32`, so both layouts can be tried without knowing which one the sender
/// used.  Candidates are accepted only when the word carries `f_seqno` and no
/// undefined bits, because upstream sets `seqno` on every packet it builds
/// (`AdnlPeerPairImpl::send_messages_from_queue`).  Returns `None` when no
/// candidate looks like a `flags:#` word.
pub(crate) fn raw_packet_flags(payload: &[u8]) -> Option<u32> {
    const PACKET_CONTENTS_ID: u32 = 0xd142cd89;
    const F_SEQNO: u32 = 0x40;
    const DEFINED_BITS: u32 = 0x1fff;

    let word = |offset: usize| -> Option<u32> {
        let head: [u8; 4] = payload.get(offset..offset + 4)?.try_into().ok()?;
        Some(u32::from_le_bytes(head))
    };
    if word(0)? != PACKET_CONTENTS_ID {
        return None;
    }
    let rand1_len = payload.get(4).copied()? as usize;
    let padded = rand1_len.div_ceil(4) * 4;
    [5 + rand1_len, 8 + rand1_len, 8 + padded]
        .into_iter()
        .find(|offset| {
            word(*offset).is_some_and(|flags| flags & DEFINED_BITS == flags && flags & F_SEQNO != 0)
        })
        .and_then(word)
}

#[cfg(test)]
mod tests;
