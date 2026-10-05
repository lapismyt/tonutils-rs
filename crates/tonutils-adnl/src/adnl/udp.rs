//! Datagram framing for ADNL sessions.
//!
//! UDP is deliberately exposed as a datagram primitive.  Handshake and peer
//! discovery remain owned by the caller because a UDP endpoint can be shared
//! by several ADNL peers and the protocol does not provide stream semantics.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use aes::cipher::{KeyIvInit, StreamCipher};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio_util::bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use tonutils_tl::tl::network::{
    AddressList, DhtMessage, DhtNodes, DhtNodesBoxed, DhtValueResult, OverlayNodes,
    OverlayNodesBoxed, OverlayQuery, PacketContents, PublicKey as TlPublicKey,
};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::crypto::{KeyPair, PublicKey};
use crate::{AdnlAddress, AdnlAesParams, AdnlCodec, AdnlError};

/// Maximum encoded ADNL datagram accepted by the native UDP helper.
pub const MAX_UDP_PACKET_SIZE: usize = 64 * 1024;

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

/// AES-CTR channel cipher used after an ADNL channel is established.
///
/// Channel packets carry a 32-byte SHA-256 digest followed by ciphertext.
/// The per-packet key and IV are derived from that digest and the channel
/// secret, matching the upstream TON `EncryptorAES`/`DecryptorAES` layout.
#[derive(Clone)]
pub struct AdnlChannelCipher {
    secret: [u8; 32],
}

impl AdnlChannelCipher {
    #[must_use]
    pub fn new(secret: [u8; 32]) -> Self {
        Self { secret }
    }

    #[must_use]
    pub fn secret(&self) -> [u8; 32] {
        self.secret
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Bytes {
        let digest: [u8; 32] = Sha256::digest(plaintext).into();
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(&self.secret[..16]);
        key[16..].copy_from_slice(&digest[16..]);
        let mut iv = [0u8; 16];
        iv[..4].copy_from_slice(&digest[..4]);
        iv[4..].copy_from_slice(&self.secret[20..]);
        let mut ciphertext = plaintext.to_vec();
        ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into())
            .apply_keystream(&mut ciphertext);
        let mut output = Vec::with_capacity(32 + ciphertext.len());
        output.extend_from_slice(&digest);
        output.extend_from_slice(&ciphertext);
        Bytes::from(output)
    }

    pub fn decrypt(&self, packet: &[u8]) -> Result<Bytes, AdnlError> {
        if packet.len() < 32 {
            return Err(AdnlError::TooShortPacket);
        }
        let digest: [u8; 32] = packet[..32]
            .try_into()
            .map_err(|_| AdnlError::TooShortPacket)?;
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(&self.secret[..16]);
        key[16..].copy_from_slice(&digest[16..]);
        let mut iv = [0u8; 16];
        iv[..4].copy_from_slice(&digest[..4]);
        iv[4..].copy_from_slice(&self.secret[20..]);
        let mut plaintext = packet[32..].to_vec();
        ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into())
            .apply_keystream(&mut plaintext);
        if !bool::from(Sha256::digest(&plaintext).as_slice().ct_eq(&digest)) {
            return Err(AdnlError::IntegrityError);
        }
        Ok(Bytes::from(plaintext))
    }
}

#[must_use]
pub fn reverse_channel_secret(mut secret: [u8; 32]) -> [u8; 32] {
    secret.reverse();
    secret
}

#[must_use]
pub fn ordered_channel_ciphers(
    local_id: [u8; 32],
    peer_id: [u8; 32],
    shared_secret: [u8; 32],
) -> (AdnlChannelCipher, AdnlChannelCipher) {
    let reversed = reverse_channel_secret(shared_secret);
    if local_id <= peer_id {
        (
            AdnlChannelCipher::new(reversed),
            AdnlChannelCipher::new(shared_secret),
        )
    } else {
        (
            AdnlChannelCipher::new(shared_secret),
            AdnlChannelCipher::new(reversed),
        )
    }
}

#[must_use]
pub fn channel_id_for_secret(secret: [u8; 32]) -> [u8; 32] {
    let mut public_key = Vec::with_capacity(36);
    public_key.extend_from_slice(&0x2dbcadd4u32.to_le_bytes());
    public_key.extend_from_slice(&secret);
    Sha256::digest(public_key).into()
}

/// Encodes and validates packets carried by an established ADNL channel.
pub struct AdnlChannelPacket {
    outbound_id: [u8; 32],
    inbound_id: [u8; 32],
    outbound: AdnlChannelCipher,
    inbound: AdnlChannelCipher,
    next_seqno: u64,
    highest_seqno: u64,
    received: VecDeque<u64>,
}

fn aes_encrypt(secret: [u8; 32], plaintext: &[u8]) -> Bytes {
    let digest: [u8; 32] = Sha256::digest(plaintext).into();
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&secret[..16]);
    key[16..].copy_from_slice(&digest[16..]);
    let iv: [u8; 16] = [&digest[..4], &secret[20..]]
        .concat()
        .try_into()
        .expect("digest prefix and secret suffix must form a 16-byte IV");
    let mut ciphertext = plaintext.to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into()).apply_keystream(&mut ciphertext);
    let mut output = Vec::with_capacity(32 + ciphertext.len());
    output.extend_from_slice(&digest);
    output.extend_from_slice(&ciphertext);
    Bytes::from(output)
}

fn aes_decrypt(secret: [u8; 32], packet: &[u8]) -> Result<Bytes, AdnlError> {
    if packet.len() < 32 {
        return Err(AdnlError::TooShortPacket);
    }
    let digest: [u8; 32] = packet[..32]
        .try_into()
        .map_err(|_| AdnlError::TooShortPacket)?;
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&secret[..16]);
    key[16..].copy_from_slice(&digest[16..]);
    let iv: [u8; 16] = [&digest[..4], &secret[20..]]
        .concat()
        .try_into()
        .map_err(|_| AdnlError::IntegrityError)?;
    let mut plaintext = packet[32..].to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new((&key).into(), (&iv).into()).apply_keystream(&mut plaintext);
    if !bool::from(Sha256::digest(&plaintext).as_slice().ct_eq(&digest)) {
        return Err(AdnlError::IntegrityError);
    }
    Ok(Bytes::from(plaintext))
}

/// Direct ADNL packet encryption used before an optional channel is ready.
pub fn encrypt_direct(remote: &PublicKey, plaintext: &[u8]) -> Bytes {
    let ephemeral = KeyPair::generate(&mut rand::rngs::OsRng);
    let encrypted = aes_encrypt(ephemeral.compute_shared_secret(remote), plaintext);
    let mut output = Vec::with_capacity(32 + encrypted.len());
    output.extend_from_slice(ephemeral.public_key.as_bytes());
    output.extend_from_slice(&encrypted);
    Bytes::from(output)
}

/// Decrypts a direct ADNL packet and returns the sender's ephemeral key.
pub fn decrypt_direct(local: &KeyPair, packet: &[u8]) -> Result<(PublicKey, Bytes), AdnlError> {
    if packet.len() < 64 {
        return Err(AdnlError::TooShortPacket);
    }
    let public = PublicKey::from_bytes(
        packet[..32]
            .try_into()
            .map_err(|_| AdnlError::InvalidPublicKey)?,
    )
    .ok_or(AdnlError::InvalidPublicKey)?;
    Ok((
        public,
        aes_decrypt(local.compute_shared_secret(&public), &packet[32..])?,
    ))
}

/// Authenticated UDP ADNL endpoint for direct packets and established channels.
pub struct AdnlUdpSession {
    socket: tokio::net::UdpSocket,
    local: KeyPair,
    remote: PublicKey,
    local_id: [u8; 32],
    remote_id: [u8; 32],
    channel: Option<AdnlChannelPacket>,
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
    /// Address list version advertised by this session.
    ///
    /// Frozen at connect time: upstream replaces the stored address list only
    /// on a strictly greater `version`, so a per-packet timestamp would make
    /// the peer's single stored source address flap between the ports of
    /// concurrent sessions to the same peer.  A session started later always
    /// wins, which keeps the peer's answer routing stable for the lifetime of
    /// a session.
    address_version: i32,
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
            channel: None,
            local_channel: KeyPair::generate(&mut rand::rngs::OsRng),
            confirm_channels: true,
            peer_channel_key: None,
            channel_requested_at: None,
            pending_channel: None,
            received: VecDeque::new(),
            peer_addr_list_version: None,
            address_version: now_i32(),
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
                version: self.address_version,
                reinit_date: local_reinit_date(),
                priority: 0,
                expire_at: 0,
            });
        }
        if contents.recv_addr_list_version.is_none() {
            contents.recv_addr_list_version = self.peer_addr_list_version;
        }
    }

    pub async fn send_contents(&mut self, contents: PacketContents) -> Result<usize, AdnlError> {
        if self.channel.is_none() {
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
            .channel
            .as_mut()
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
        self.channel = None;
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
        let via_channel = self.channel.as_ref().is_some_and(|channel| {
            size >= channel.inbound_id.len()
                && packet[..channel.inbound_id.len()] == channel.inbound_id
        });
        if via_channel {
            let channel = self
                .channel
                .as_mut()
                .expect("channel presence was checked above");
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
            let channel_ids = self
                .channel
                .as_ref()
                .map(|channel| {
                    format!(
                        " in_id={} out_id={}",
                        hex::encode(channel.inbound_id),
                        hex::encode(channel.outbound_id)
                    )
                })
                .unwrap_or_default();
            if self.confirm_channels && self.channel.is_none() {
                // The prefix is neither ours nor a channel we hold, so the
                // peer most likely still uses a channel negotiated by an
                // earlier session of ours.  Ask it to re-key.
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
                    if self.peer_channel_key == Some(key.0) && self.channel.is_some() {
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
                    if self.peer_channel_key == Some(key.0) && self.channel.is_some() {
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
        self.channel = Some(AdnlChannelPacket::new_directional(
            outbound_id,
            inbound_id,
            outbound,
            inbound,
        ));
        Ok(())
    }

    /// Sends `adnl.message.createChannel` for this session's local channel key.
    ///
    /// Used both to initiate a channel and to re-key a channel the peer still
    /// holds from an earlier session of ours: upstream only accepts the new key
    /// when the carried `date` is strictly greater than the date it recorded,
    /// which is why the current time is used here.
    async fn request_channel(&mut self) -> Result<(), AdnlError> {
        const REQUEST_INTERVAL: Duration = Duration::from_secs(10);
        if self
            .channel_requested_at
            .is_some_and(|at| at.elapsed() < REQUEST_INTERVAL)
        {
            return Ok(());
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
        self.send_direct_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::CreateChannel {
                key: Int256(self.local_channel.public_key.to_bytes()),
                date,
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
        Ok(())
    }

    pub async fn establish_channel(&mut self, timeout: Duration) -> Result<(), AdnlError> {
        const MAX_RETRIES: u32 = 3;
        if self.channel.is_some() {
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
            while self.channel.is_none() {
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
            if self.channel.is_some() {
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

    #[allow(clippy::unnecessary_join)]
    pub async fn dht_find_node(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindNode { key, k: count });
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findNode",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    let nodes: DhtNodesBoxed = tl_proto::deserialize(&answer)
                        .map_err(|error| AdnlError::MalformedPacket(error.to_string()))?;
                    let now = now_i32();
                    let total = nodes.nodes.len();
                    let selected: Vec<_> = nodes
                        .nodes
                        .into_iter()
                        .filter(|node| node.is_valid(now))
                        .collect();
                    log::debug!(
                        "dht_find_node: received {total} raw nodes, {} passed is_valid",
                        selected.len(),
                    );
                    return Ok(DhtNodesBoxed { nodes: selected });
                }
            }
        }
    }

    pub async fn dht_find_value(
        &mut self,
        key: Int256,
        count: i32,
        timeout: Duration,
    ) -> Result<DhtValueResult, AdnlError> {
        let query_id = Int256::random();
        let query = tl_proto::serialize(DhtMessage::FindValue { key, k: count });
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "DHT findValue",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    // Try to deserialize as DhtValueResult first
                    match tl_proto::deserialize::<DhtValueResult>(&answer) {
                        Ok(result) => return Ok(result),
                        Err(value_err) => {
                            // Log raw bytes for debugging
                            let hex_full = hex::encode(&answer);
                            log::debug!(
                                "dht_find_value: DhtValueResult deserialize failed ({}), \
                                 answer len={}, hex_full={hex_full}, \
                                 trying DhtNodesBoxed fallback",
                                value_err,
                                answer.len()
                            );

                            // Step-by-step: try to read constructor ID
                            if answer.len() >= 4 {
                                let ctor = u32::from_le_bytes(answer[..4].try_into().unwrap());
                                log::debug!(
                                    "dht_find_value: step-by-step: constructor=0x{ctor:08x} \
                                     (expected Found=0xe40cf774, NotFound=0xa2620568)"
                                );
                                if ctor == 0xe40cf774 && answer.len() >= 8 {
                                    let inner_ctor =
                                        u32::from_le_bytes(answer[4..8].try_into().unwrap());
                                    log::debug!(
                                        "dht_find_value: step-by-step: inner constructor=0x{inner_ctor:08x} \
                                         (expected DhtValue=0x90ad27cb)"
                                    );
                                }
                            }

                            // Fallback: try DhtNodesBoxed (some nodes respond with
                            // dht.Nodes constructor 0x7974a0be instead of DhtValueResult)
                            if let Ok(nodes_boxed) = tl_proto::deserialize::<DhtNodesBoxed>(&answer)
                            {
                                log::debug!(
                                    "dht_find_value: got DhtNodesBoxed fallback with {} nodes",
                                    nodes_boxed.nodes.len()
                                );
                                return Ok(DhtValueResult::NotFound {
                                    nodes: DhtNodes {
                                        nodes: nodes_boxed.nodes,
                                    },
                                });
                            }

                            // Both failed, return the original error with hex context
                            return Err(AdnlError::MalformedPacket(format!(
                                "DhtValueResult={value_err}, hex_full={hex_full}"
                            )));
                        }
                    }
                }
            }
        }
    }

    pub async fn overlay_get_random_peers(
        &mut self,
        overlay: Int256,
        timeout: Duration,
    ) -> Result<OverlayNodesBoxed, AdnlError> {
        let query_id = Int256::random();
        self.send_overlay_get_random_peers_with_id(overlay, query_id.clone())
            .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(AdnlError::Timeout {
                    operation: "overlay getRandomPeers",
                    timeout,
                });
            }
            let packet = self.recv_timeout(remaining).await?;
            let messages = packet
                .message
                .into_iter()
                .chain(packet.messages.into_iter().flatten());
            for message in messages {
                if let AdnlMessage::Answer {
                    query_id: id,
                    answer,
                } = message
                    && id == query_id
                {
                    return tl_proto::deserialize(&answer)
                        .map_err(|error| AdnlError::MalformedPacket(error.to_string()));
                }
            }
        }
    }

    pub async fn send_overlay_get_random_peers(
        &mut self,
        overlay: Int256,
    ) -> Result<usize, AdnlError> {
        self.send_overlay_get_random_peers_with_id(overlay, Int256::random())
            .await
    }

    async fn send_overlay_get_random_peers_with_id(
        &mut self,
        overlay: Int256,
        query_id: Int256,
    ) -> Result<usize, AdnlError> {
        let mut query = tl_proto::serialize(OverlayQuery::Query {
            overlay: overlay.clone(),
        });
        query.extend(tl_proto::serialize(OverlayQuery::GetRandomPeers {
            peers: OverlayNodes {
                nodes: vec![self.local_overlay_node(overlay)],
            },
        }));
        self.send_contents(PacketContents {
            rand1: vec![0; 7],
            flags: (),
            from: None,
            from_short: None,
            message: Some(AdnlMessage::Query {
                query_id: query_id.clone(),
                query,
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
        .await
    }

    /// Returns this session's signed `overlay.node` record for `overlay`.
    ///
    /// The record carries no address: overlay peers resolve the ADNL address
    /// through the DHT `address` value for our node id.  It is what
    /// `overlay.getRandomPeers` answers must contain, see
    /// `adnl.message.query` handling in the overlay session.
    pub fn local_overlay_node(&self, overlay: Int256) -> tonutils_tl::tl::network::OverlayNode {
        let version = now_i32();
        let to_sign = tonutils_tl::tl::network::OverlayNodeToSign {
            id: tonutils_tl::tl::network::AdnlIdShort {
                id: Int256(self.local_id),
            },
            overlay: overlay.clone(),
            version,
        };
        tonutils_tl::tl::network::OverlayNode {
            id: tonutils_tl::tl::network::PublicKey::Ed25519 {
                key: Int256(self.local.public_key.to_bytes()),
            },
            overlay,
            version,
            signature: self.local.sign_raw(&tl_proto::serialize(to_sign)).to_vec(),
        }
    }
}

impl AdnlChannelPacket {
    #[must_use]
    pub fn new(
        channel_id: [u8; 32],
        outbound: AdnlChannelCipher,
        inbound: AdnlChannelCipher,
    ) -> Self {
        Self::new_directional(channel_id, channel_id, outbound, inbound)
    }

    #[must_use]
    pub fn new_directional(
        outbound_id: [u8; 32],
        inbound_id: [u8; 32],
        outbound: AdnlChannelCipher,
        inbound: AdnlChannelCipher,
    ) -> Self {
        Self {
            outbound_id,
            inbound_id,
            outbound,
            inbound,
            next_seqno: 0,
            highest_seqno: 0,
            received: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn channel_id(&self) -> [u8; 32] {
        self.outbound_id
    }

    /// Encrypts `contents` for an established channel.
    ///
    /// `seqno` and `confirm_seqno` are assigned from the channel's own
    /// counters when the caller left them unset.  [`AdnlUdpSession`] always
    /// sets them first, because the peer pair - not the channel - owns that
    /// counter; standalone users keep the channel-local behaviour.
    pub fn encode(&mut self, mut contents: PacketContents) -> Result<Bytes, AdnlError> {
        if contents.message.is_none() && contents.messages.is_none() {
            return Err(AdnlError::InvalidPacket);
        }
        if contents.seqno.is_none() {
            self.next_seqno = self.next_seqno.saturating_add(1);
            contents.seqno = Some(self.next_seqno);
        }
        if contents.confirm_seqno.is_none() {
            contents.confirm_seqno = Some(self.highest_seqno);
        }
        let payload = tl_proto::serialize(contents);
        let encrypted = self.outbound.encrypt(&payload);
        if encrypted.len() + self.outbound_id.len() > MAX_UDP_PACKET_SIZE {
            return Err(AdnlError::TooLongPacket);
        }
        let mut packet = Vec::with_capacity(self.outbound_id.len() + encrypted.len());
        packet.extend_from_slice(&self.outbound_id);
        packet.extend_from_slice(&encrypted);
        Ok(Bytes::from(packet))
    }

    pub fn decode(&mut self, datagram: &[u8]) -> Result<PacketContents, AdnlError> {
        if datagram.len() < self.inbound_id.len() + 32
            || datagram.len() > MAX_UDP_PACKET_SIZE
            || datagram[..self.inbound_id.len()] != self.inbound_id
        {
            return Err(AdnlError::InvalidPacket);
        }
        let payload = self.inbound.decrypt(&datagram[self.inbound_id.len()..])?;
        let contents: PacketContents = tl_proto::deserialize(&payload).map_err(|error| {
            let prefix = hex::encode(payload.iter().take(96).copied().collect::<Vec<_>>());
            AdnlError::MalformedPacket(format!("{error} (channel payload={prefix})"))
        })?;
        if let Some(confirm_seqno) = contents.confirm_seqno
            && confirm_seqno > self.next_seqno
        {
            return Err(AdnlError::ReplayDetected);
        }
        if let Some(seqno) = contents.seqno {
            if seqno == 0
                || self.received.contains(&seqno)
                || (self.highest_seqno > 4096 && seqno + 4096 < self.highest_seqno)
            {
                return Err(AdnlError::ReplayDetected);
            }
            self.highest_seqno = self.highest_seqno.max(seqno);
            self.received.push_back(seqno);
            while self.received.len() > 4096 {
                self.received.pop_front();
            }
        }
        Ok(contents)
    }
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
#[cfg(test)]
#[path = "udp_tests.rs"]
mod tests;
