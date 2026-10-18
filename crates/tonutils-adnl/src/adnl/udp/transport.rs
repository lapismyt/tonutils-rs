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
//! The transport is deliberately additive: [`AdnlUdpSession::connect`]
//! keeps its signature and still gives every call its own connected
//! socket, so existing callers keep working unchanged, while callers
//! that need one session per peer adopt [`AdnlUdpTransport::for_node`],
//! the process-wide transport cache keyed by node id and local
//! address.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub(super) queue: mpsc::Sender<Vec<u8>>,
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
        let local_id = AdnlAddress::from(&local.public_key).to_bytes();
        let (idle, _) = watch::channel(false);
        let inner = Arc::new(TransportInner {
            socket,
            local,
            local_id,
            sessions: Mutex::new(HashMap::new()),
            routes: Mutex::new(Routes::default()),
            demux: Mutex::new(DemuxState::default()),
            sent_datagrams: AtomicU64::new(0),
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
        tokio::spawn(async move {
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
    /// Channel packets carry their 32-byte channel id in the clear,
    /// so they route by the registry sessions publish on channel
    /// install.  Direct packets carry the destination ADNL id in the
    /// clear; anything addressed to this node routes by the source
    /// address to the session for that peer.  Everything else is
    /// dropped: it is either addressed to another node or from a peer
    /// this node has no session for.
    ///
    /// Enqueueing never waits.  Session queues are bounded, and a
    /// datagram for a session whose consumer is behind is dropped:
    /// waiting here would stall this single receive task and add
    /// head-of-line delay to every other session on the shared
    /// socket, while a drop costs only that session's datagram -
    /// the loss mode UDP already has.
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
        let mut datagrams: u64 = 0;
        let mut dropped: u64 = 0;
        let mut closed: u64 = 0;
        let mut unrouted: u64 = 0;
        loop {
            if self.drain_if_idle() {
                return;
            }
            tokio::select! {
                result = self.socket.recv_from(&mut packet) => {
                    let Ok((size, peer)) = result else {
                        self.demux.lock().unwrap().running = false;
                        return;
                    };
                    datagrams += 1;
                    if size > MAX_UDP_PACKET_SIZE {
                        continue;
                    }
                    let route = {
                        let routes = self.routes.lock().unwrap();
                        let mut route = None;
                        if let Ok(prefix) = <[u8; 32]>::try_from(&packet[..size.min(32)]) {
                            route = routes.by_channel_id.get(&prefix).cloned();
                            if route.is_none() && prefix == self.local_id {
                                route = routes.by_peer_addr.get(&peer).cloned();
                            }
                        }
                        route
                    };
                    // A raw exchange installs its hook before sending, and a
                    // server address never routes to a session, so a packet
                    // the demultiplexer would otherwise drop is handed to the
                    // waiter that sent for it.
                    let mut captured = false;
                    if route.is_none()
                        && let Some(exchange) = self.raw.lock().unwrap().as_ref()
                        && peer.ip() == exchange.from_ip
                        && exchange.tx.try_send(packet[..size].to_vec()).is_ok()
                    {
                        captured = true;
                        // The waiter returns after `try_send` succeeds, so an
                        // empty slot cannot fail.  The hook stays in place and
                        // `raw_exchange` clears it once it has been observed.
                        log::trace!("ADNL transport captured raw reply from {peer}");
                    }
                    if let Some(route) = route {
                        match route.queue.try_send(packet[..size].to_vec()) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                dropped += 1;
                                // The session's consumer is behind.  Blocking
                                // this task until it drains would delay every
                                // other session on the socket.
                                log::debug!(
                                    "ADNL transport dropping datagram: session {} queue full",
                                    route.session
                                );
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                closed += 1;
                                // The session is gone; its Drop impl already
                                // removed its routes, so this entry is stale.
                                log::trace!(
                                    "ADNL transport route to session {} closed",
                                    route.session
                                );
                            }
                        }
                    } else if !captured {
                        unrouted += 1;
                    }
                }
                _ = idle.changed() => {}
                () = tokio::time::sleep_until(heartbeat) => {
                    let sent = self.sent_datagrams.load(Ordering::Relaxed);
                    log::debug!(
                        "ADNL transport rx heartbeat: datagrams={datagrams} sent={} dropped={dropped} closed={closed} unrouted={unrouted} dt_ms={} {}",
                        sent - last_sent,
                        last_beat.elapsed().as_millis(),
                        match self.socket.local_addr().ok().and_then(kernel_socket_stats) {
                            Some((queued, kernel_drops)) => {
                                format!("queue_bytes={queued} kernel_drops={kernel_drops}")
                            }
                            None => "queue_bytes=- kernel_drops=-".to_string(),
                        },
                    );
                    last_sent = sent;
                    last_beat = std::time::Instant::now();
                    heartbeat = tokio::time::Instant::now() + DEMUX_HEARTBEAT;
                    datagrams = 0;
                    dropped = 0;
                    closed = 0;
                    unrouted = 0;
                }
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
