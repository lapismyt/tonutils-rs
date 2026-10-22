//! State machine for one authenticated ADNL session over UDP.
//!
//! The transport half lives here; the query half (`dht_find_node`,
//! `overlay_get_random_peers`, and friends) is a second inherent block in
//! [`session::query`] so both files stay readable.  The receive, send,
//! and channel-control halves are [`session::receive`],
//! [`session::send`], and [`session::control`].
//!
//! A session comes in two transport variants, see [`Source`]: a
//! session from [`AdnlUdpSession::connect`] owns a connected socket
//! of its own and is driven by its consumer, while a session from
//! [`AdnlUdpTransport::session_for`] shares the single unconnected
//! socket of its transport, whose receive task routes each datagram
//! to the session that owns the channel id it carries or the peer
//! address it arrived from.  Both variants exist once per peer, are
//! cheap to clone, and every method takes `&self`: the mutable
//! per-peer state lives behind [`SessionInner::state`], decoded
//! packets reach the caller through the session's inbox, and answers
//! to in-flight queries are routed by query id straight to the
//! waiting call instead of competing for the inbox.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tonutils_tl::tl::network::{AddressList, DhtMessage, PacketContents, PublicKey as TlPublicKey};
use tonutils_tl::{Int256, Message as AdnlMessage};

use crate::crypto::{KeyPair, PublicKey};
use crate::{AdnlAddress, AdnlError};

use super::transport::{Route, TransportInner};
use super::{
    AdnlChannelPacket, MAX_SESSION_CHANNELS, MAX_TRACKED_QUERIES, MAX_UDP_PACKET_SIZE,
    REQUEST_CHANNEL_INTERVAL, channel_id_for_secret, decrypt_direct, encrypt_direct,
    highest_received_seqno, local_reinit_date, message_kind, message_vector, next_outgoing_seqno,
    note_our_addr_version, note_peer_holds_our_addr, note_peer_reinit_date, now_i32,
    ordered_channel_ciphers, our_addr_view, outgoing_seqno, peer_reinit_date, raw_packet_flags,
    record_received_seqno,
};

mod control;
mod diag;
mod query;
mod receive;
mod send;

/// Raw datagrams buffered for one session before the transport's
/// receive task applies backpressure.
const SESSION_QUEUE: usize = 64;

/// Inbound queries remembered per session by the instant their
/// datagram was read off the socket, for the answer-latency
/// diagnostic.
///
/// One peer cannot have more unanswered queries of this kind pending
/// than it sends as a burst, so a small ring is enough to attach the
/// socket-read instant to the answer that leaves later.
const MAX_INBOUND_QUERIES: usize = 256;

/// Queries whose waiter is gone keep their registration instant for
/// this many most-recent entries, so an answer that arrives after the
/// caller gave up can still be logged with its real age.
const MAX_RECENT_QUERIES: usize = 4096;

/// One in-flight query: the instant its waiter was registered and the
/// channel that delivers the answer to that waiter.
type PendingQuery = (std::time::Instant, tokio::sync::oneshot::Sender<Vec<u8>>);

/// One inbound query remembered for the answer-latency diagnostic: its
/// query id, the instant its datagram was read off the socket, and the
/// TL constructor the peer asked with (the little-endian id word at
/// the start of the query payload, `0` when the payload is too short).
type InboundQuery = ([u8; 32], std::time::Instant, u32);

/// One registered synchronous answer for inbound queries, keyed by the
/// query's outer TL constructor word (see
/// [`AdnlUdpSession::register_query_answer`]).
///
/// The closure receives the query payload and returns the answer
/// payload, or `None` to decline an id it shares but does not
/// recognize; a declined query continues to the consumer unchanged.
type FastAnswer = Arc<dyn Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync>;

/// One authenticated ADNL-over-UDP peer.
///
/// Cloning shares the same underlying session: all clones observe the
/// same channels, sequence numbers, and inbox, which is what makes a
/// single session per peer id safe to hand to every caller that talks
/// to that peer.
#[derive(Clone)]
pub struct AdnlUdpSession {
    inner: Arc<SessionInner>,
}

/// Shared state of one session, visible to every clone.
pub(crate) struct SessionInner {
    id: u64,
    /// Where this session's datagrams travel.
    source: Source,
    local: KeyPair,
    local_id: [u8; 32],
    remote_id: [u8; 32],
    remote: PublicKey,
    remote_addr: Mutex<SocketAddr>,
    /// Whether this session may establish ADNL channels.  One-shot
    /// sessions (DHT/overlay lookups) disable it: they are dropped
    /// right after the query, and a peer that keeps the negotiated
    /// channel for as long as it holds our ADNL node id.  Every later
    /// session would then receive channel packets it cannot decrypt.
    confirm_channels: AtomicBool,
    /// Mutable per-peer state: channels, sequence bookkeeping, and the
    /// replay window.  Held only for synchronous critical sections and
    /// the datagram send, never across a consumer's receive wait.
    state: tokio::sync::Mutex<SessionState>,
    /// Raw datagrams routed here by the transport's receive task, each
    /// with the instant the receive thread read it off the socket.
    ///
    /// The session's own receive task is the only consumer, so it holds
    /// the lock for the whole loop; nothing else ever waits on it.
    queue: tokio::sync::Mutex<mpsc::Receiver<(std::time::Instant, Vec<u8>)>>,
    queue_tx: mpsc::Sender<(std::time::Instant, Vec<u8>)>,
    /// Decoded packets for the session's consumer.
    inbox: tokio::sync::Mutex<mpsc::Receiver<PacketContents>>,
    inbox_tx: mpsc::Sender<PacketContents>,
    /// Consumer-bound packets the receive loop dropped because the
    /// inbox was full and one packet was already parked: during
    /// backpressure the loop keeps reading datagrams so their
    /// registered answers leave on time, and a second packet that
    /// needs the consumer is dropped with UDP loss semantics.
    inbox_drops: AtomicU64,
    /// In-flight queries by query id, so an answer is routed to the
    /// call that asked for it even when several callers share the
    /// session.  Each entry keeps the instant the waiter was
    /// registered so a delivered answer can log its real age.
    pending: Mutex<HashMap<[u8; 32], PendingQuery>>,
    /// Queries whose waiter is gone (the caller timed out, or the send
    /// failed), by query id with their registration instants, newest
    /// last and bounded to [`MAX_RECENT_QUERIES`].  An answer for one
    /// of them arrives after the caller stopped waiting, so the age
    /// diagnostic picks it up from here.
    recent: Mutex<VecDeque<([u8; 32], std::time::Instant)>>,
    /// Inbound queries by query id with the instant their datagram was
    /// read off the socket and the TL constructor they asked with,
    /// newest last and bounded to [`MAX_INBOUND_QUERIES`].
    ///
    /// [`AdnlUdpSession::send_answer`] removes the entry, so its log
    /// line reports how long the peer's query waited inside this
    /// process before the answer left - the leg a peer's
    /// `overlay.ping` deadline is measured against.
    inbound_queries: Mutex<VecDeque<InboundQuery>>,
    /// Synchronous answers for inbound queries, by the query's outer TL
    /// constructor word, registered through
    /// [`AdnlUdpSession::register_query_answer`].
    ///
    /// The session's receive task consults this before delivering a
    /// query to the consumer, so an answered query never queues behind
    /// the consumer's inbox and the consumer never sees it.
    fast_answers: Mutex<HashMap<u32, FastAnswer>>,
}

/// Where a session's datagrams travel.
///
/// A session created by [`AdnlUdpSession::connect`] owns a
/// connected socket of its own, released synchronously when the
/// session is dropped.  A session handed out by
/// [`AdnlUdpTransport::session_for`] instead reads from the queue
/// of a shared transport, whose single socket is demultiplexed by
/// the transport's task and released only once the transport's
/// last session is gone.
enum Source {
    /// A socket connected to the peer, owned by this session alone.
    Solo(tokio::net::UdpSocket),
    /// One session of a shared [`AdnlUdpTransport`].
    Shared(Arc<TransportInner>),
}

impl Source {
    /// Sends a packet to the peer's current source address.
    ///
    /// A solo socket is connected, so the address is implicit; a
    /// shared socket is unconnected and addressed per datagram.
    async fn send_to(&self, packet: &[u8], addr: SocketAddr) -> Result<usize, AdnlError> {
        match self {
            Source::Solo(socket) => Ok(socket.send(packet).await?),
            Source::Shared(transport) => {
                // Sends issued on the inbound executor thread take the
                // synchronous non-blocking path: the tokio socket's
                // readiness registration belongs to the application
                // runtime, so an awaited send that finds the kernel
                // send buffer full would wait for a driver this
                // runtime does not poll.  A full buffer is ordinary
                // UDP loss and is counted as such.
                let sent = if super::inbound::on_inbound_thread() {
                    transport
                        .socket
                        .try_send_to(packet, addr)
                        .inspect_err(|error| {
                            if error.kind() == std::io::ErrorKind::WouldBlock {
                                transport.tx_drops.fetch_add(1, Ordering::Relaxed);
                            }
                        })?
                } else {
                    transport.socket.send_to(packet, addr).await?
                };
                transport.sent_datagrams.fetch_add(1, Ordering::Relaxed);
                Ok(sent)
            }
        }
    }

    /// Local address of the underlying socket.
    fn local_addr(&self) -> Result<SocketAddr, AdnlError> {
        let socket = match self {
            Source::Solo(socket) => socket,
            Source::Shared(transport) => &transport.socket,
        };
        socket.local_addr().map_err(AdnlError::from)
    }
}

/// Mutable per-peer state of a session.
///
/// Everything the peer validates or that a re-key invalidates lives
/// here; the pair-wide counters ([`next_outgoing_seqno`] and friends)
/// stay process-wide, see [`super::PeerPairState`].
struct SessionState {
    /// Negotiated ADNL channels for this peer, newest first.
    ///
    /// The first entry is used for sending.  Earlier channels stay so
    /// that a re-key does not discard datagrams the peer already put on
    /// the wire, and so that a channel negotiated by an earlier round
    /// of this session keeps working after the peer switches back to it.
    channels: Vec<AdnlChannelPacket>,
    /// Local ADNL channel key for this session.  It is generated once
    /// and reused for every `adnl.message.createChannel` /
    /// `adnl.message.confirmChannel` this session sends, so repeated
    /// negotiation rounds stay idempotent instead of re-keying a channel
    /// the peer has already accepted.
    local_channel: KeyPair,
    /// Peer channel public key the current channel was derived from.
    peer_channel_key: Option<[u8; 32]>,
    /// Date of the last channel request sent by this session, used to
    /// rate limit re-key attempts.
    channel_requested_at: Option<std::time::Instant>,
    /// When set, this session has sent `createChannel` and awaits the
    /// peer's `confirmChannel`.  The value is the date that was sent.
    pending_channel: Option<i32>,
    /// Local replay window.  Sequence numbers of the peer pair itself
    /// are process-wide; only the duplicates-inside-this-session guard
    /// is session state.
    received: VecDeque<u64>,
    /// Version of the address list last received from the peer, echoed
    /// back as `recv_addr_list_version` so the peer knows we keep its
    /// address.
    peer_addr_list_version: Option<i32>,
    /// Ids of the `adnl.message.query` packets this session has sent,
    /// oldest first, so an answer addressed to a sibling session of the
    /// same ADNL node id can be told apart from a reply to this one.
    sent_queries: VecDeque<[u8; 32]>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            channels: Vec::new(),
            local_channel: KeyPair::generate(&mut rand::rngs::OsRng),
            peer_channel_key: None,
            channel_requested_at: None,
            pending_channel: None,
            received: VecDeque::new(),
            peer_addr_list_version: None,
            sent_queries: VecDeque::new(),
        }
    }
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
        // A solo session needs no receive task: its consumer's
        // receive wait reads, decodes, and returns packets
        // directly, and the socket is released synchronously
        // once the session is dropped.
        let inner = Arc::new(SessionInner::solo(socket, &local, remote));
        Ok(Self { inner })
    }

    pub async fn connect_with_channel(
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
        local: KeyPair,
        remote: PublicKey,
        timeout: Duration,
    ) -> Result<Self, AdnlError> {
        let session = Self::connect(local_addr, remote_addr, local, remote).await?;
        session.establish_channel(timeout).await?;
        Ok(session)
    }

    /// Enables or disables ADNL channel establishment for this session.
    ///
    /// Sessions used for a single query should pass `false`: the session
    /// is dropped right after the answer, while the peer keeps the
    /// negotiated channel for as long as it holds our ADNL node id.
    /// Every later session would then receive channel packets it cannot
    /// decrypt.
    ///
    /// Defaults to `true`.
    pub fn set_confirm_channels(&self, enabled: bool) {
        self.inner
            .confirm_channels
            .store(enabled, Ordering::Relaxed);
    }

    /// The peer's current source address.
    ///
    /// Starts as the address the session was dialled with and follows the
    /// transport's route table whenever the peer's datagrams arrive from a
    /// newer source address, so it stays correct across NAT rebinding and
    /// peer mobility - unlike the address the caller dialled.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        *self.inner.remote_addr.lock().unwrap()
    }

    /// Starts ADNL channel negotiation without waiting for the answer.
    ///
    /// Sends `adnl.message.createChannel` so that the peer confirms a
    /// channel this session can actually decrypt.  A session that starts
    /// without one is otherwise permanently deaf: the peer keeps using
    /// the channel it negotiated with an earlier session of the same
    /// ADNL node id, whose local channel key this session no longer
    /// holds, and every datagram it sends is dropped as an unknown
    /// prefix.
    ///
    /// Unlike [`Self::establish_channel`] this does not block, so a
    /// caller can bundle its first query with the handshake instead of
    /// paying a round trip before any application traffic.
    ///
    /// Does nothing for one-shot sessions (see [`Self::set_confirm_channels`])
    /// and when a channel is already established; repeated attempts are
    /// rate limited to once per ten seconds.
    pub async fn initiate_channel(&self) -> Result<(), AdnlError> {
        let message = {
            let mut state = self.inner.state.lock().await;
            self.inner.create_channel_message(&mut state)
        };
        let Some(message) = message else {
            return Ok(());
        };
        self.inner
            .send_contents(PacketContents {
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

    /// Assigns the peer pair's shared sequence numbers to an outgoing
    /// packet and fills the address fields the peer needs to reach us.
    ///
    /// Direct and channel packets share one sequence number space per
    /// peer pair, so both paths allocate from [`super::PeerPairState`]
    /// here instead of from any session-local counter.
    pub async fn send_contents(&self, contents: PacketContents) -> Result<usize, AdnlError> {
        self.inner.send_contents(contents).await
    }

    /// Sends an answer to an inbound query and logs how long that
    /// query waited inside this process since its datagram was read.
    ///
    /// A peer's `overlay.ping`/`dht.ping` answer is what grants this
    /// node liveness on that peer (upstream allows five seconds, see
    /// `OverlayImpl::check_pending_peer` in `overlay/overlay-peers.cpp`)
    /// and what admits it into the peer's broadcast fan-out, so this
    /// latency decides whether pushes ever arrive.  The value is
    /// logged even when the send itself fails: it measures the wait,
    /// not the outcome.
    pub async fn send_answer(&self, query_id: Int256, answer: Vec<u8>) -> Result<usize, AdnlError> {
        self.inner.send_answer_to(&query_id, answer).await
    }

    /// Consumer-bound packets this session's receive loop dropped
    /// because its inbox was full while another packet was already
    /// parked for it.
    ///
    /// This is the loss side of the registered-answer lane: while the
    /// consumer stops draining its inbox, the receive loop keeps
    /// reading datagrams so their registered answers leave inside the
    /// peer's admission window, and a second consumer-bound packet
    /// seen during that window is dropped with UDP loss semantics.
    #[must_use]
    pub fn inbox_drops(&self) -> u64 {
        self.inner.inbox_drops.load(Ordering::Relaxed)
    }

    /// Registers a synchronous answer for inbound queries whose outer
    /// TL constructor word is `id`, replacing any earlier registration.
    ///
    /// The closure runs on the session's receive task - the layer that
    /// already authenticated the datagram - so an answered query is
    /// sent back there and never reaches [`Self::recv_contents`].  A
    /// query the closure declines with `None` takes the ordinary
    /// consumer path untouched, which lets one registration cover a
    /// wrapper id whose inner constructors need different handling.
    /// The closure must not call back into this session.
    ///
    /// This exists because a peer's liveness probes are answered on a
    /// deadline (five seconds, `OverlayImpl::check_pending_peer`), and
    /// an answer the consumer builds can queue behind application work
    /// for longer than that: the live gate measured `overlay.ping`
    /// answers taking seconds while the application runtime was busy.
    /// The receive loop keeps servicing these registrations even while
    /// the consumer's inbox is applying backpressure (see
    /// [`Self::inbox_drops`]).
    pub fn register_query_answer(
        &self,
        id: u32,
        answer: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) {
        self.inner
            .fast_answers
            .lock()
            .unwrap()
            .insert(id, Arc::new(answer));
    }

    /// Receives the next decoded packet for this session's consumer.
    ///
    /// Packets are decoded by the session's receive task, which also
    /// applies the peer-pair checks and routes answers to in-flight
    /// queries; this call returns whatever else arrives, exactly like
    /// the socket receive it replaces.
    /// Receives the next decoded packet for this session's
    /// consumer.
    ///
    /// A shared session is served by its receive task, so this
    /// call pops the decoded packet off the session's inbox.  A
    /// solo session has no task of its own - its consumer's
    /// receive wait is the receive loop - so this call reads,
    /// decodes, and validates the next datagram directly, and
    /// any answer it carries is also routed to the call waiting
    /// on that query id.
    pub async fn recv_contents(&self) -> Result<PacketContents, AdnlError> {
        match &self.inner.source {
            Source::Shared(_) => {
                let mut inbox = self.inner.inbox.lock().await;
                inbox.recv().await.ok_or(AdnlError::EndOfStream)
            }
            Source::Solo(_) => {
                // The hour-long period is only a polling
                // interval: a solo session's consumer drives
                // its own receive loop, so a period that
                // elapses without a valid packet simply
                // starts the next one.
                loop {
                    match self
                        .inner
                        .recv_solo_timeout(Duration::from_secs(3600))
                        .await
                    {
                        Ok(Some(contents)) => return Ok(contents),
                        Ok(None) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }

    pub async fn send_timeout(
        &self,
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

    pub async fn recv_timeout(&self, timeout: Duration) -> Result<PacketContents, AdnlError> {
        tokio::time::timeout(timeout, self.recv_contents())
            .await
            .map_err(|_| AdnlError::Timeout {
                operation: "ADNL UDP packet receive",
                timeout,
            })?
    }

    /// Establishes an ADNL channel with the peer.
    ///
    /// Sends `createChannel` together with a signed address list
    /// query, whose answer doubles as proof that the peer can
    /// reach this session's source address, and retries with
    /// exponential backoff until the peer's `confirmChannel`
    /// arrives.  The confirmation is processed by the session's
    /// receive task, so the loop here only has to drain whatever
    /// else arrives while it waits.
    pub async fn establish_channel(&self, timeout: Duration) -> Result<(), AdnlError> {
        const MAX_RETRIES: u32 = 3;
        {
            let state = self.inner.state.lock().await;
            if !state.channels.is_empty() {
                return Ok(());
            }
        }
        let mut attempt = 0u32;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            attempt += 1;
            let date = now_i32();
            let local_channel_key = {
                let mut state = self.inner.state.lock().await;
                state.pending_channel = Some(date);
                state.local_channel.public_key.to_bytes()
            };
            self.inner
                .send_contents(PacketContents {
                    rand1: vec![0; 7],
                    flags: (),
                    from: None,
                    from_short: None,
                    message: None,
                    messages: Some(vec![
                        AdnlMessage::CreateChannel {
                            key: Int256(local_channel_key),
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
            while {
                let state = self.inner.state.lock().await;
                state.channels.is_empty()
            } {
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
            {
                let state = self.inner.state.lock().await;
                if !state.channels.is_empty() {
                    return Ok(());
                }
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
}

impl SessionInner {
    /// Creates a session on its own connected socket.
    ///
    /// The socket is released as soon as the last clone of the
    /// session is dropped, so the local address can be rebound
    /// immediately by the next session.
    fn solo(socket: tokio::net::UdpSocket, local: &KeyPair, remote: PublicKey) -> Self {
        let local_id = AdnlAddress::from(&local.public_key).to_bytes();
        let remote_id = AdnlAddress::from(&remote).to_bytes();
        let (queue_tx, queue) = mpsc::channel(SESSION_QUEUE);
        let (inbox_tx, inbox) = mpsc::channel(SESSION_QUEUE);
        Self {
            id: next_session_id(),
            source: Source::Solo(socket),
            local: *local,
            local_id,
            remote_id,
            remote,
            remote_addr: Mutex::new(
                "127.0.0.1:0"
                    .parse()
                    .expect("loopback fallback address parses"),
            ),
            inbox_drops: AtomicU64::new(0),
            confirm_channels: AtomicBool::new(true),
            state: tokio::sync::Mutex::new(SessionState::new()),
            queue: tokio::sync::Mutex::new(queue),
            queue_tx,
            inbox: tokio::sync::Mutex::new(inbox),
            inbox_tx,
            pending: Mutex::new(HashMap::new()),
            recent: Mutex::new(VecDeque::new()),
            inbound_queries: Mutex::new(VecDeque::new()),
            fast_answers: Mutex::new(HashMap::new()),
        }
    }

    /// Creates a session on a shared transport.
    ///
    /// The caller registers the initial address route and starts the
    /// receive task.
    pub(super) fn shared(
        transport: Arc<TransportInner>,
        remote_id: [u8; 32],
        remote_addr: SocketAddr,
        remote: PublicKey,
    ) -> Self {
        transport.add_session();
        let (queue_tx, queue) = mpsc::channel(SESSION_QUEUE);
        let (inbox_tx, inbox) = mpsc::channel(SESSION_QUEUE);
        Self {
            id: next_session_id(),
            source: Source::Shared(transport.clone()),
            local: transport.local,
            local_id: transport.local_id,
            remote_id,
            remote,
            remote_addr: Mutex::new(remote_addr),
            inbox_drops: AtomicU64::new(0),
            confirm_channels: AtomicBool::new(true),
            state: tokio::sync::Mutex::new(SessionState::new()),
            queue: tokio::sync::Mutex::new(queue),
            queue_tx,
            inbox: tokio::sync::Mutex::new(inbox),
            inbox_tx,
            pending: Mutex::new(HashMap::new()),
            recent: Mutex::new(VecDeque::new()),
            inbound_queries: Mutex::new(VecDeque::new()),
            fast_answers: Mutex::new(HashMap::new()),
        }
    }

    /// Registers this session's source address route with the transport.
    ///
    /// Solo sessions have no routes: their connected socket already
    /// receives only from the peer.
    pub(super) fn register_route(&self, addr: SocketAddr) {
        let Source::Shared(transport) = &self.source else {
            return;
        };
        let mut routes = transport.routes.lock().unwrap();
        routes.by_peer_addr.insert(
            addr,
            Route {
                session: self.id,
                queue: self.queue_tx.clone(),
            },
        );
    }

    /// Adopts a new source address for the peer, replacing the route
    /// the old address carried.
    pub(super) fn adopt_address(&self, addr: SocketAddr) {
        let Source::Shared(transport) = &self.source else {
            return;
        };
        let mut current = self.remote_addr.lock().unwrap();
        if *current == addr {
            return;
        }
        let mut routes = transport.routes.lock().unwrap();
        routes.by_peer_addr.remove(&*current);
        routes.by_peer_addr.insert(
            addr,
            Route {
                session: self.id,
                queue: self.queue_tx.clone(),
            },
        );
        *current = addr;
    }

    /// Registers a negotiated channel id with the transport so channel
    /// packets route here even after the peer changes its source
    /// address.
    fn register_channel_route(&self, channel_id: [u8; 32]) {
        let Source::Shared(transport) = &self.source else {
            return;
        };
        let mut routes = transport.routes.lock().unwrap();
        routes.by_channel_id.insert(
            channel_id,
            Route {
                session: self.id,
                queue: self.queue_tx.clone(),
            },
        );
    }

    /// Starts the session's receive task.
    ///
    /// Only shared sessions have one: their datagrams arrive
    /// through the transport's demultiplexer, and the task is
    /// what decodes them and routes answers to in-flight
    /// queries.  A solo session is driven by its consumer -
    /// [`AdnlUdpSession::recv_contents`] reads, decodes, and
    /// returns one packet per call, exactly the pre-transport
    /// behavior - and its socket is released synchronously when
    /// the session is dropped.
    ///
    /// The task runs on the process-wide inbound executor
    /// ([`super::inbound`]), not the runtime that created the session:
    /// decoding, answer routing, and registered fast answers must not
    /// queue behind the application's own ready queue.
    pub(super) fn spawn(self: Arc<Self>) {
        let Source::Shared(_) = &self.source else {
            return;
        };
        super::inbound::handle().spawn(async move {
            self.receive_loop().await;
        });
    }
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        // Only shared sessions leave routes behind: a stale
        // channel route would swallow datagrams for a peer this
        // process no longer talks to, and a stale address route
        // would drop the peer's direct packets.  Solo sessions
        // have no routes - their connected socket already
        // receives only from the peer - and their socket is
        // released by the field drop itself.
        let Source::Shared(transport) = &self.source else {
            return;
        };
        let mut routes = transport.routes.lock().unwrap();
        routes
            .by_channel_id
            .retain(|_, route| route.session != self.id);
        routes
            .by_peer_addr
            .retain(|_, route| route.session != self.id);
        drop(routes);
        // When the last session of the transport is gone, wake
        // the demultiplexing task so it drains and releases the
        // shared socket.
        transport.drop_session();
    }
}

/// Monotonic session ids, used only to clean up stale routes.
fn next_session_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl From<Arc<SessionInner>> for AdnlUdpSession {
    fn from(inner: Arc<SessionInner>) -> Self {
        Self { inner }
    }
}
