//! One ADNL UDP socket shared by every session of one node id.
//!
//! Upstream keeps a single socket per ADNL node and demultiplexes
//! incoming datagrams by ADNL channel id and by peer address
//! (`AdnlNetworkManager` in `adnl/adnl-network-manager.cpp`).  This
//! crate previously gave every [`AdnlUdpSession`](super::AdnlUdpSession)
//! its own connected socket, so a one-shot lookup socket advertised
//! the same node id on a second local port and outbid the live
//! session's address list version, leaving peers with a dead port in
//! their connection table.  [`AdnlUdpTransport`] restores the upstream
//! model: all sessions of one node id share one unconnected socket,
//! and the transport's receive task routes each datagram to the
//! session that owns the channel id it carries or the peer address it
//! arrived from.
//!
//! Reading the socket is deliberately not done by that async task: a
//! dedicated OS thread drains the kernel receive queue into a bounded
//! channel, because tokio pulls its IO driver only every
//! `event_interval` (61) task polls or when the run queue empties, and
//! a busy runtime can leave that window open for tens of seconds.  The
//! channel's send wakes the routing task through the ordinary task
//! waker, so inbound latency does not depend on the driver's cadence.
//! See `TransportInner::run` and the `receive_loop` function.
//!
//! The transport is deliberately additive: [`AdnlUdpSession::connect`]
//! keeps its signature and still gives every call its own connected
//! socket, so existing callers keep working unchanged, while callers
//! that need one session per peer adopt [`AdnlUdpTransport::for_node`],
//! the process-wide transport cache keyed by node id and local
//! address.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

use crate::crypto::{KeyPair, PublicKey};
use crate::{AdnlAddress, AdnlError};

use super::MAX_UDP_PACKET_SIZE;
use super::session::SessionInner;

/// How often the demultiplexer logs its receive-side counters.
const DEMUX_HEARTBEAT: Duration = Duration::from_secs(10);

/// Capacity of the socket-to-demultiplexer channel.
///
/// The receive thread never waits: a full channel drops the datagram
/// (the loss mode UDP already has) and counts it as an overrun.  The
/// capacity is sized to smooth multi-hundred-datagram bursts without
/// letting the demultiplexer fall far behind; a scheduling window of a
/// contended runtime produces at most a few hundred datagrams.
const DEMUX_CHANNEL_CAPACITY: usize = 1024;

/// How long the receive thread naps while the socket has nothing to
/// read.
///
/// The thread exists so that draining the kernel receive queue does
/// not depend on the async runtime's IO driver, which tokio only pulls
/// every `event_interval` (61) task polls or when the run queue
/// empties.  A nap of one millisecond keeps that thread idle-cheap
/// while inbound latency stays in the single-digit-millisecond range,
/// far below the seconds-scale budgets of the live probes.
const RECV_IDLE_NAP: Duration = Duration::from_millis(1);

/// Shared ADNL UDP endpoint for one local node id.
///
/// Cloning shares the socket and the session table, so a process
/// keeps exactly one transport per ADNL node id.
#[derive(Clone)]
pub struct AdnlUdpTransport {
    inner: Arc<TransportInner>,
}

pub(super) struct TransportInner {
    pub(super) socket: UdpSocket,
    /// Nonblocking duplicate of the descriptor behind `socket` that the
    /// receive thread drains.
    ///
    /// `O_NONBLOCK` belongs to the open file description, so the
    /// duplicate and the async sends share it.  Reading the kernel
    /// queue from this descriptor keeps inbound latency independent of
    /// the runtime's IO driver cadence.
    recv_socket: std::net::UdpSocket,
    pub(super) local: KeyPair,
    pub(super) local_id: [u8; 32],
    /// Live sessions by remote ADNL id.  Weak handles keep the table
    /// from owning sessions: the last caller that drops a session
    /// lets its routes be cleaned up by the session's `Drop` impl.
    sessions: Mutex<HashMap<[u8; 32], Weak<SessionInner>>>,
    /// Datagram routes: channel id and peer address to a session.
    pub(super) routes: Mutex<Routes>,
    /// Demultiplexer lifecycle, serialized against `session_for`.
    demux: Mutex<DemuxState>,
    /// Datagrams this socket sent since the transport was created,
    /// for the receive heartbeat's send-rate column.
    pub(super) sent_datagrams: AtomicU64,
    /// Datagrams the receive thread dropped because the
    /// socket-to-demultiplexer channel was full.
    rx_overruns: Arc<AtomicU64>,
    /// Longest wait between a datagram being read off the socket and
    /// the demultiplexer routing it, in milliseconds.
    ///
    /// This is the inbound leg the transport owns; the waiter's own
    /// runtime wake after the route handoff is not part of it.  The
    /// heartbeat reports the cumulative maximum, which outlives the
    /// per-beat column, and the busy-runtime regression test asserts
    /// on it.
    dispatch_wait_max_ms: AtomicU64,
    /// Set when every handle of this transport is gone, so the receive
    /// thread exits even if the demultiplexing task was dropped
    /// without running its own cleanup (runtime shutdown).
    shutdown: Arc<AtomicBool>,
    /// One in-flight raw (non-ADNL) exchange, if any.
    raw: Mutex<Option<RawExchange>>,
    /// Set when the last session of this transport is dropped, so
    /// the demultiplexing task wakes up and releases the socket.
    idle: watch::Sender<bool>,
}

/// A raw request in flight on the shared socket.
///
/// The demultiplexer drops every datagram it cannot route, so an
/// exchange installs this hook: the next datagram from `from_ip` is
/// handed to `tx` instead of being discarded.
struct RawExchange {
    /// Source address the awaited reply comes from.
    from_ip: std::net::IpAddr,
    /// Bearer for the one reply the exchange is waiting for.
    tx: mpsc::Sender<Vec<u8>>,
}

#[derive(Default)]
struct DemuxState {
    /// Whether the demultiplexing task is running.
    running: bool,
    /// Sessions currently alive on this transport.
    live: usize,
}

#[derive(Default)]
pub(super) struct Routes {
    pub(super) by_channel_id: HashMap<[u8; 32], Route>,
    pub(super) by_peer_addr: HashMap<SocketAddr, Route>,
}

#[derive(Clone)]
pub(super) struct Route {
    pub(super) session: u64,
    /// Raw datagrams with the instant the receive thread read them off
    /// the socket, so a decoded query can report how long it waited
    /// inside this process before its answer left.
    pub(super) queue: mpsc::Sender<(std::time::Instant, Vec<u8>)>,
}

impl Drop for TransportInner {
    fn drop(&mut self) {
        // The receive thread holds only a duplicate descriptor and this
        // flag.  Setting it here releases the thread even when the
        // demultiplexing task is dropped without running its cleanup,
        // which is what a runtime shutdown does.
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl AdnlUdpTransport {
    /// Binds the shared socket.
    ///
    /// The socket stays unconnected: the transport's task decides
    /// per datagram which session it belongs to.  The task itself
    /// starts with the first session ([`Self::session_for`]), so
    /// a transport that never serves a session releases its socket
    /// as soon as its last handle is dropped.
    pub async fn bind(local_addr: SocketAddr, local: KeyPair) -> Result<Self, AdnlError> {
        let socket = UdpSocket::bind(local_addr).await?;
        // Split the socket into one handle for the async sends and one
        // duplicate the receive thread drains.  `try_clone` duplicates
        // the descriptor, so both handles share the receive queue and
        // the source address, and `O_NONBLOCK` belongs to the open file
        // description, so the duplicate is nonblocking too.
        let std_socket = socket.into_std()?;
        let recv_socket = std_socket.try_clone()?;
        let socket = UdpSocket::from_std(std_socket)?;
        let local_id = AdnlAddress::from(&local.public_key).to_bytes();
        let (idle, _) = watch::channel(false);
        let inner = Arc::new(TransportInner {
            socket,
            recv_socket,
            local,
            local_id,
            sessions: Mutex::new(HashMap::new()),
            routes: Mutex::new(Routes::default()),
            demux: Mutex::new(DemuxState::default()),
            sent_datagrams: AtomicU64::new(0),
            rx_overruns: Arc::new(AtomicU64::new(0)),
            dispatch_wait_max_ms: AtomicU64::new(0),
            shutdown: Arc::new(AtomicBool::new(false)),
            raw: Mutex::new(None),
            idle,
        });
        Ok(Self { inner })
    }

    /// Returns the transport every session of this node id and
    /// local address shares, binding it on first use.
    ///
    /// Upstream keeps one socket per ADNL node id
    /// (`AdnlNetworkManager` in `adnl/adnl-network-manager.cpp`)
    /// and pytoniq keeps a single `AdnlTransport` for the same
    /// reason: a second socket for the same node id advertises a
    /// second source address, while a peer keeps exactly one
    /// address per node id and replaces it only on a strictly
    /// greater `adnl.addressList.version`.  Sharing one socket is
    /// therefore what keeps a one-shot lookup from outbidding a
    /// live session's address list and leaving the peer with a
    /// dead port in its connection table.
    ///
    /// Concurrent callers serialize on the cache lock, which is
    /// held across the bind, so exactly one socket is bound per
    /// key.  The cache holds weak handles and drops expired
    /// entries, so a node id whose sessions and handles are all
    /// gone releases its socket and binds fresh on the next use.
    pub async fn for_node(local_addr: SocketAddr, local: KeyPair) -> Result<Self, AdnlError> {
        let local_id = AdnlAddress::from(&local.public_key).to_bytes();
        let key = (local_id, local_addr);
        let mut transports = node_transports().lock().await;
        if let Some(inner) = transports.get(&key).and_then(Weak::upgrade) {
            return Ok(Self { inner });
        }
        transports.retain(|_, cached| Weak::upgrade(cached).is_some());
        let transport = Self::bind(local_addr, local).await?;
        transports.insert(key, Arc::downgrade(&transport.inner));
        Ok(transport)
    }

    /// Local address the shared socket is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, AdnlError> {
        self.inner.socket.local_addr().map_err(AdnlError::from)
    }

    /// Longest wait observed between a datagram being read off the
    /// socket and the demultiplexer routing it, in milliseconds, since
    /// the transport was created.
    ///
    /// Routing runs on the process-wide inbound executor, so this value
    /// stays small no matter how busy the sessions' own runtime is.
    /// The busy-runtime regression test asserts on it: it is the
    /// inbound leg this crate owns, while the wake of whatever waiter
    /// the route hands a datagram to belongs to that waiter's runtime.
    #[cfg(test)]
    pub(super) fn dispatch_wait_max_ms(&self) -> u64 {
        self.inner.dispatch_wait_max_ms.load(Ordering::Relaxed)
    }

    /// Sends one raw, non-ADNL datagram and returns the reply from `peer`.
    ///
    /// [`Self::session_for`] starts the demultiplexer, which routes every
    /// datagram it can match to a session and drops the rest.  A raw
    /// exchange instead waits on the shared socket itself, so it reports
    /// the address that socket is seen from rather than the address a
    /// separate probe socket would get: NAT maps each socket separately.
    /// That distinction is what makes the result safe to publish as this
    /// node's `dht.address` value.
    ///
    /// Only one exchange is in flight at a time, and the hook is removed
    /// on completion, so an ADNL packet that happens to arrive while an
    /// exchange waits is still routed normally.
    pub async fn raw_exchange(
        &self,
        peer: SocketAddr,
        request: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, AdnlError> {
        // Only the demultiplexer reads the shared socket, so a transport
        // that has not served a session yet must start it to see a reply.
        self.inner.start_demux();
        let (tx, mut rx) = mpsc::channel(1);
        {
            let mut raw = self.inner.raw.lock().unwrap();
            if raw.is_some() {
                return Err(AdnlError::MalformedPacket(
                    "another raw exchange is already in flight".to_string(),
                ));
            }
            *raw = Some(RawExchange {
                from_ip: peer.ip(),
                tx,
            });
        }
        let result = match self.inner.socket.send_to(request, peer).await {
            Err(error) => Err(AdnlError::from(error)),
            Ok(_) => match tokio::time::timeout(timeout, rx.recv()).await {
                Ok(Some(reply)) => Ok(reply),
                Ok(None) => Err(AdnlError::MalformedPacket(
                    "raw exchange waiter was dropped".to_string(),
                )),
                Err(_) => Err(AdnlError::Timeout {
                    operation: "raw exchange",
                    timeout,
                }),
            },
        };
        *self.inner.raw.lock().unwrap() = None;
        result
    }

    /// Local ADNL node id every session of this transport shares.
    pub fn local_id(&self) -> [u8; 32] {
        self.inner.local_id
    }

    /// Local keypair every session of this transport shares.
    pub fn local_key(&self) -> &KeyPair {
        &self.inner.local
    }

    /// Returns the session for `remote_id`, creating it on first use.
    ///
    /// All callers that talk to one peer id get the same session, so
    /// the peer sees exactly one source address and one address list
    /// version for this node id.  A session whose peer moved to a new
    /// address adopts the address on the next `session_for` call; that
    /// deliberate repoint belongs to the caller that owns the peer's
    /// steady-state traffic.  Dial-only callers use
    /// [`Self::session_for_probe`] instead, which never repoints.
    pub fn session_for(
        &self,
        remote_id: [u8; 32],
        remote_addr: SocketAddr,
        remote: PublicKey,
    ) -> super::AdnlUdpSession {
        self.session_for_inner(remote_id, remote_addr, remote, true)
    }

    /// Returns the session for `remote`, deriving its ADNL id.
    pub fn session_for_peer(
        &self,
        remote: PublicKey,
        remote_addr: SocketAddr,
    ) -> super::AdnlUdpSession {
        let remote_id = AdnlAddress::from(&remote).to_bytes();
        self.session_for(remote_id, remote_addr, remote)
    }

    /// Returns the peer's session without changing the address it
    /// sends to.
    ///
    /// Lookups and probes dial many candidate endpoints of the same
    /// peer, and a peer keys its answers to the source address it saw
    /// last, so the session needs one stable, manager-chosen address:
    /// adopting a candidate here would repoint the manager's keepalives
    /// and broadcasts at an endpoint that may be stale, and one
    /// concurrent sweep over the recorded endpoints could leave the
    /// whole session pinned to a dead one.  Probes therefore carry
    /// their own destination per query, see
    /// [`AdnlUdpSession::overlay_get_random_peers_to`](super::AdnlUdpSession::overlay_get_random_peers_to).
    /// A peer without a session yet gets one bound to `remote_addr`,
    /// the only address known for it.
    pub fn session_for_probe(
        &self,
        remote: PublicKey,
        remote_addr: SocketAddr,
    ) -> super::AdnlUdpSession {
        let remote_id = AdnlAddress::from(&remote).to_bytes();
        self.session_for_inner(remote_id, remote_addr, remote, false)
    }

    /// Shared implementation of [`Self::session_for`] and
    /// [`Self::session_for_probe`]: upgrades the cached session for
    /// `remote_id` or spawns one bound to `remote_addr`.  `adopt`
    /// repoints an existing session at `remote_addr`.
    fn session_for_inner(
        &self,
        remote_id: [u8; 32],
        remote_addr: SocketAddr,
        remote: PublicKey,
        adopt: bool,
    ) -> super::AdnlUdpSession {
        self.inner.start_demux();
        let mut sessions = self.inner.sessions.lock().unwrap();
        if let Some(session) = sessions.get(&remote_id).and_then(Weak::upgrade) {
            if adopt {
                session.adopt_address(remote_addr);
            }
            return session.into();
        }
        let session = Arc::new(SessionInner::shared(
            self.inner.clone(),
            remote_id,
            remote_addr,
            remote,
        ));
        session.register_route(remote_addr);
        session.clone().spawn();
        sessions.insert(remote_id, Arc::downgrade(&session));
        session.into()
    }
}

/// Transport handles of one process, keyed by node id and
/// local address.
type NodeTransports = HashMap<([u8; 32], SocketAddr), Weak<TransportInner>>;

/// Process-wide transports, keyed by node id and local address.
///
/// The weak handles follow the transport's own reference
/// counting: an entry expires once every session and every
/// handle of that node id is gone.
fn node_transports() -> &'static tokio::sync::Mutex<NodeTransports> {
    static TRANSPORTS: std::sync::OnceLock<tokio::sync::Mutex<NodeTransports>> =
        std::sync::OnceLock::new();
    TRANSPORTS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

impl TransportInner {
    /// Registers a new live session on this transport.
    pub(super) fn add_session(&self) {
        self.demux.lock().unwrap().live += 1;
    }

    /// Unregisters a session, waking the demultiplexer when the last
    /// one is gone so it can release the shared socket.
    pub(super) fn drop_session(&self) {
        let wake = {
            let mut demux = self.demux.lock().unwrap();
            demux.live = demux.live.saturating_sub(1);
            demux.live == 0
        };
        if wake {
            let _ = self.idle.send_replace(true);
        }
    }

    /// Starts the demultiplexing task if it is not running.
    ///
    /// Called by every `session_for`: the task drains once the
    /// last session of this transport is dropped - releasing
    /// the socket - and a session created afterwards restarts
    /// it here.  A transport that never serves a session never
    /// starts a task, so its socket is released with the
    /// transport itself.
    fn start_demux(self: &Arc<Self>) {
        let mut demux = self.demux.lock().unwrap();
        if demux.running {
            // A draining task re-checks the idle flag under the same
            // lock, so clearing it here keeps the task alive for the
            // session that is about to be created.
            let _ = self.idle.send_replace(false);
            return;
        }
        demux.running = true;
        let _ = self.idle.send_replace(false);
        let inner = self.clone();
        super::inbound::handle().spawn(async move {
            inner.run().await;
        });
    }

    /// Decides whether the demultiplexing task should drain.
    ///
    /// The decision is made under the lifecycle lock so it cannot race
    /// a concurrent `session_for`: either the task stops and clears
    /// `running` - letting the next `session_for` restart it - or the
    /// session start already cleared the idle flag and the task keeps
    /// serving the new session.
    fn drain_if_idle(self: &Arc<Self>) -> bool {
        let mut demux = self.demux.lock().unwrap();
        if *self.idle.borrow() {
            demux.running = false;
            true
        } else {
            false
        }
    }

    /// Demultiplexes every incoming datagram to its session.
    ///
    /// Routing and the never-wait enqueueing rules live in
    /// [`Self::dispatch`]; this loop moves datagrams from the receive
    /// thread's channel - or from the socket itself, when the thread
    /// could not start - into it, and logs the heartbeat.
    ///
    /// The socket is drained by a dedicated thread, not by this task:
    /// tokio pulls its IO driver only every `event_interval` (61) task
    /// polls or when the run queue empties, and a runtime that stays
    /// busy with CPU-heavy polls - eight scanner samples on four shared
    /// CI vCPUs - left that window open for tens of seconds.  Datagrams
    /// then sat in the kernel receive queue long after they arrived and
    /// every probe answered late.  The thread forwards datagrams over a
    /// bounded channel whose send wakes this task through the ordinary
    /// task waker, so inbound routing no longer waits for a driver
    /// turn.
    ///
    /// Every [`DEMUX_HEARTBEAT`] the task logs what it read, sent,
    /// dropped, and failed to route in the interval, how long the
    /// interval really was in wall-clock terms, and - on Linux - how
    /// many bytes the kernel still holds unread for the socket and how
    /// many datagrams it dropped for overflow.  Together with the
    /// per-answer age log in the session receive path, that separates
    /// "this process read the answer late" from "the answer arrived
    /// late", which line order in a log alone cannot.
    async fn run(self: Arc<Self>) {
        let mut idle = self.idle.subscribe();
        let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
        let mut heartbeat = tokio::time::Instant::now() + DEMUX_HEARTBEAT;
        let mut last_beat = std::time::Instant::now();
        let mut last_sent = self.sent_datagrams.load(Ordering::Relaxed);
        let mut last_overruns = self.rx_overruns.load(Ordering::Relaxed);
        let mut stats = DemuxStats::default();
        let mut iters: u64 = 0;
        let mut gap_max_ms: u128 = 0;
        let mut last_iter = std::time::Instant::now();
        // A receive thread that cannot be started falls back to reading
        // the socket from this task, which keeps routing correct (at
        // the cost of the driver-cadence latency the thread exists to
        // avoid).
        let (tx, mut rx) =
            mpsc::channel::<(std::time::Instant, Vec<u8>, SocketAddr)>(DEMUX_CHANNEL_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let use_thread = match self.recv_socket.try_clone() {
            Ok(socket) => {
                let tx = tx.clone();
                let stop = stop.clone();
                let shutdown = self.shutdown.clone();
                let overruns = self.rx_overruns.clone();
                match std::thread::Builder::new()
                    .name("adnl-udp-recv".to_string())
                    .spawn(move || receive_loop(socket, tx, stop, shutdown, overruns))
                {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("ADNL transport receive thread failed to start: {error}");
                        false
                    }
                }
            }
            Err(error) => {
                log::warn!("ADNL transport receive socket clone failed: {error}");
                false
            }
        };
        // Only the thread's clone may keep the channel open: a dead
        // thread must be visible as an exhausted receiver instead of a
        // pending one.
        drop(tx);
        loop {
            let iter_now = std::time::Instant::now();
            let gap = iter_now.duration_since(last_iter).as_millis();
            if gap > gap_max_ms {
                gap_max_ms = gap;
            }
            last_iter = iter_now;
            iters += 1;
            if self.drain_if_idle() {
                break;
            }
            tokio::select! {
                // The channel drain is deliberately unconstrained: a
                // channel receive charges cooperative budget, and a
                // budget-exhausted receive parks the task on the
                // runtime's deferred list, which is only flushed at a
                // park - the same driver window this thread exists to
                // bypass.  The channel is bounded, so one unconstrained
                // drain stays small.
                maybe = tokio::task::unconstrained(rx.recv()), if use_thread => {
                    match maybe {
                        Some((at, datagram, peer)) => {
                            self.dispatch(at, &datagram, peer, &mut stats);
                        }
                        None => {
                            // The receive thread exited on its own; stop so
                            // the next `session_for` restarts the demux.
                            log::error!("ADNL transport receive thread stopped unexpectedly");
                            self.demux.lock().unwrap().running = false;
                            return;
                        }
                    }
                }
                result = self.socket.recv_from(&mut packet), if !use_thread => {
                    let Ok((size, peer)) = result else {
                        self.demux.lock().unwrap().running = false;
                        return;
                    };
                    // The fallback reads the socket in this task, so its
                    // read is driver-gated: stamp the iteration start so
                    // that gate is part of the dispatch-wait diagnostic.
                    self.dispatch(iter_now, &packet[..size], peer, &mut stats);
                }
                _ = idle.changed() => {}
                () = tokio::time::sleep_until(heartbeat) => {
                    let sent = self.sent_datagrams.load(Ordering::Relaxed);
                    let overruns = self.rx_overruns.load(Ordering::Relaxed);
                    log::debug!(
                        "ADNL transport rx heartbeat: datagrams={} sent={} dropped={} closed={} unrouted={} overrun={} wait_max_ms={} wait_cum_max_ms={} iters={iters} gap_max_ms={gap_max_ms} dt_ms={} {}",
                        stats.datagrams,
                        sent - last_sent,
                        stats.dropped,
                        stats.closed,
                        stats.unrouted,
                        overruns - last_overruns,
                        stats.wait_max_ms,
                        self.dispatch_wait_max_ms.load(Ordering::Relaxed),
                        last_beat.elapsed().as_millis(),
                        match self.socket.local_addr().ok().and_then(kernel_socket_stats) {
                            Some((queued, kernel_drops)) => {
                                format!("queue_bytes={queued} kernel_drops={kernel_drops}")
                            }
                            None => "queue_bytes=- kernel_drops=-".to_string(),
                        },
                    );
                    last_sent = sent;
                    last_overruns = overruns;
                    last_beat = std::time::Instant::now();
                    heartbeat = tokio::time::Instant::now() + DEMUX_HEARTBEAT;
                    stats = DemuxStats::default();
                    iters = 0;
                    gap_max_ms = 0;
                }
            }
        }
        // The demultiplexer drained; release the receive thread.  It
        // observes the flag within one nap.
        stop.store(true, Ordering::Relaxed);
    }

    /// Routes one datagram to the session queue or raw-exchange hook
    /// that owns it and counts the outcome.
    ///
    /// Channel packets carry their 32-byte channel id in the clear, so
    /// they route by the registry sessions publish on channel install.
    /// Direct packets carry the destination ADNL id in the clear;
    /// anything addressed to this node routes by the source address to
    /// the session for that peer.  Everything else is dropped: it is
    /// either addressed to another node or from a peer this node has no
    /// session for.
    ///
    /// Enqueueing never waits.  Session queues are bounded, and a
    /// datagram for a session whose consumer is behind is dropped:
    /// waiting would add head-of-line delay to every other session on
    /// the shared socket, while a drop costs only that session's
    /// datagram - the loss mode UDP already has.
    ///
    /// `at` is the instant the receive thread read the datagram off
    /// the socket; it travels with the packet so the session that
    /// answers a query can report the whole in-process wait, and its
    /// distance to now is the receive task's own dispatch delay.
    fn dispatch(
        &self,
        at: std::time::Instant,
        packet: &[u8],
        peer: SocketAddr,
        stats: &mut DemuxStats,
    ) {
        stats.datagrams += 1;
        let wait_ms = at.elapsed().as_millis();
        if wait_ms > stats.wait_max_ms {
            stats.wait_max_ms = wait_ms;
        }
        self.dispatch_wait_max_ms
            .fetch_max(wait_ms as u64, Ordering::Relaxed);
        if packet.len() > MAX_UDP_PACKET_SIZE {
            return;
        }
        let route = {
            let routes = self.routes.lock().unwrap();
            let mut route = None;
            if let Ok(prefix) = <[u8; 32]>::try_from(&packet[..packet.len().min(32)]) {
                route = routes.by_channel_id.get(&prefix).cloned();
                if route.is_none() && prefix == self.local_id {
                    route = routes.by_peer_addr.get(&peer).cloned();
                }
            }
            route
        };
        // A raw exchange installs its hook before sending, and a server
        // address never routes to a session, so a packet the
        // demultiplexer would otherwise drop is handed to the waiter
        // that sent for it.
        let mut captured = false;
        if route.is_none()
            && let Some(exchange) = self.raw.lock().unwrap().as_ref()
            && peer.ip() == exchange.from_ip
            && exchange.tx.try_send(packet.to_vec()).is_ok()
        {
            captured = true;
            // The waiter returns after `try_send` succeeds, so an empty
            // slot cannot fail.  The hook stays in place and
            // `raw_exchange` clears it once it has been observed.
            log::trace!("ADNL transport captured raw reply from {peer}");
        }
        if let Some(route) = route {
            match route.queue.try_send((at, packet.to_vec())) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    stats.dropped += 1;
                    // The session's consumer is behind.  Blocking this
                    // task until it drains would delay every other
                    // session on the socket.
                    log::debug!(
                        "ADNL transport dropping datagram: session {} queue full",
                        route.session
                    );
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    stats.closed += 1;
                    // The session is gone; its Drop impl already removed
                    // its routes, so this entry is stale.
                    log::trace!("ADNL transport route to session {} closed", route.session);
                }
            }
        } else if !captured {
            stats.unrouted += 1;
        }
    }
}

/// Receive-side counters of one demultiplexer heartbeat interval.
#[derive(Default)]
struct DemuxStats {
    /// Datagrams the receive thread forwarded to the demultiplexer.
    datagrams: u64,
    /// Datagrams dropped because a session queue was full.
    dropped: u64,
    /// Datagrams whose session queue was already closed.
    closed: u64,
    /// Datagrams that matched no session route and no raw exchange.
    unrouted: u64,
    /// Longest wait between the receive thread reading a datagram off
    /// the socket and this task dispatching it, in milliseconds.
    ///
    /// The receive thread wakes this task through the channel, so this
    /// column is the async runtime's own scheduling delay on the
    /// inbound path - the first leg of the `ADNL answer latency` a
    /// session logs when it answers a peer's query.
    wait_max_ms: u128,
}

/// Drains the shared socket into the demultiplexer channel.
///
/// Runs on its own thread so the kernel receive queue is read as soon
/// as datagrams arrive: the async runtime only pulls its IO driver
/// every `event_interval` (61) task polls, and a runtime busy with
/// CPU-heavy polls can leave that window open for tens of seconds,
/// piling datagrams up in the kernel buffer behind an unread socket.
/// The thread owns a nonblocking duplicate of the socket descriptor,
/// takes no locks, and never waits on the demultiplexer: a full channel
/// drops the datagram - the loss mode UDP already has - and counts it.
/// Every datagram is stamped with the instant it was read, so the
/// session that later answers a query can report the whole in-process
/// wait from the socket to the answer leaving.
///
/// It exits when the demultiplexer drains (`stop`) or when every handle
/// of the transport is gone (`shutdown`), so a runtime that shuts down
/// without running the demultiplexer's cleanup still releases it.
fn receive_loop(
    socket: std::net::UdpSocket,
    tx: mpsc::Sender<(std::time::Instant, Vec<u8>, SocketAddr)>,
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    overruns: Arc<AtomicU64>,
) {
    let mut packet = vec![0u8; MAX_UDP_PACKET_SIZE + 1];
    while !stop.load(Ordering::Relaxed) && !shutdown.load(Ordering::Relaxed) {
        match socket.recv_from(&mut packet) {
            Ok((size, peer)) => {
                let at = std::time::Instant::now();
                if tx.try_send((at, packet[..size].to_vec(), peer)).is_err() {
                    overruns.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(RECV_IDLE_NAP);
            }
            Err(error) => {
                log::warn!("ADNL transport receive error: {error}");
                std::thread::sleep(RECV_IDLE_NAP);
            }
        }
    }
}

/// The kernel's view of one UDP socket: bytes still queued unread in
/// its receive buffer, and datagrams it dropped for lack of room.
///
/// Read from `/proc/net/udp` on Linux and reported as `None` on other
/// platforms and for IPv6 sockets.  The demultiplexer's heartbeat uses
/// it to tell datagrams this process sat on from datagrams that
/// arrived late.  A kernel that does not print the trailing drop
/// column is reported as zero drops.
#[cfg(target_os = "linux")]
fn kernel_socket_stats(local: SocketAddr) -> Option<(u64, u64)> {
    let SocketAddr::V4(v4) = local else {
        return None;
    };
    let ip = v4.ip().octets();
    let needle = format!(
        "{:02X}{:02X}{:02X}{:02X}:{:04X}",
        ip[3],
        ip[2],
        ip[1],
        ip[0],
        v4.port()
    );
    let table = std::fs::read_to_string("/proc/net/udp").ok()?;
    for line in table.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 6 || !fields[1].eq_ignore_ascii_case(&needle) {
            continue;
        }
        // Field 4 is `tx_queue:rx_queue`, both hex, on modern kernels;
        // older layouts print `tx_queue` and `rx_queue` as two
        // whitespace-separated fields.
        let queued = if fields[4].contains(':') {
            u64::from_str_radix(fields[4].split(':').nth(1)?, 16).ok()?
        } else {
            u64::from_str_radix(fields.get(5)?, 16).ok()?
        };
        let drops = fields
            .get(12)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        return Some((queued, drops));
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn kernel_socket_stats(_local: SocketAddr) -> Option<(u64, u64)> {
    None
}
