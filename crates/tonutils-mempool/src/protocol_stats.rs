//! Process-wide overlay protocol counters.
//!
//! The counters are global (not per [`crate::MempoolScanner`]) because an
//! overlay session is owned by the peer pool and is not reachable from the
//! scanner. They exist so that a live run can explain *why* the node stayed
//! silent without requiring `RUST_LOG`:
//!
//! * [`ProtocolStats::queries_received`] / [`ProtocolStats::pongs_sent`] show
//!   whether peers verify this node with `overlay.ping` at all;
//! * [`ProtocolStats::queries_wrapped`] counts queries that arrived inside
//!   the `overlay.query` header that upstream always adds
//!   (`OverlayManager::send_query` in `overlay/overlay-manager.cpp`);
//! * [`ProtocolStats::membership_listed`] counts `overlay.getRandomPeers`
//!   answers that contain this node, i.e. peers that treat it as a verified
//!   member and therefore broadcast to it;
//! * [`ProtocolStats::address_storing_peers`] /
//!   [`ProtocolStats::unique_pingers`] /
//!   [`ProtocolStats::membership_listing_peers`] separate the admission
//!   stages per distinct peer: a peer that stored a routable address for this
//!   node can originate `overlay.ping`, a peer that pinged answered our pong,
//!   and a peer that listed us treats us as a verified member;
//! * [`ProtocolStats::pushers`] closes the funnel: distinct peers whose
//!   custom overlay payloads reached the scanner, i.e. peers that actually
//!   push to this node after admitting it;
//! * [`ProtocolStats::dht_pings_received`] counts pytoniq-style liveness
//!   probes (`dht.ping` wrapped in `overlay.query`), which a peer sends only
//!   for nodes it keeps in its own peer list;
//! * [`ProtocolStats::fec_parts_sent`] /
//!   [`ProtocolStats::fec_repair_parts_sent`] separate source from repair
//!   traffic on the send side, and [`ProtocolStats::fec_symbols_accepted`] /
//!   [`ProtocolStats::fec_repair_symbols_received`] /
//!   [`ProtocolStats::fec_decoded`] /
//!   [`ProtocolStats::fec_param_rejections`] explain the receive side: do
//!   FEC parts arrive at all, does repair traffic flow, do reassemblies
//!   complete, and is someone sending crafted parameters.
//!
//! All getters return a point-in-time [`ProtocolStats`] snapshot.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

/// Cap on the distinct-peer dedup sets behind
/// [`ProtocolStats::unique_pingers`] and
/// [`ProtocolStats::membership_listing_peers`].
///
/// The mainnet mempool overlay holds a few hundred members, so the sets
/// saturate only in pathological churn and a saturated set keeps counting
/// totals while the distinct-peer figure pins at the cap.
const MAX_TRACKED_PEERS: usize = 4096;

/// Snapshot of the overlay protocol counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProtocolStats {
    /// `adnl.message.query` payloads handed to the overlay query handler.
    pub queries_received: u64,
    /// Queries that carried the `overlay.query` / `overlay.queryWithExtra`
    /// header and therefore had to be unwrapped first.
    pub queries_wrapped: u64,
    /// Queries rejected before dispatch, e.g. wrapped for another overlay.
    pub queries_rejected: u64,
    /// Queries the node had no handler for.
    pub queries_unhandled: u64,
    /// Constructor id of the last unhandled query (0 when none was seen).
    pub last_unhandled_query_id: u32,
    /// `overlay.pong` answers sent.
    pub pongs_sent: u64,
    /// Total `overlay.ping` queries answered.  Distinct pingers are in
    /// [`ProtocolStats::unique_pingers`]; the pair with
    /// [`ProtocolStats::pongs_sent`] shows whether every ping produced its
    /// pong.
    pub pings_received: u64,
    /// `overlay.getRandomPeers` answers sent.
    pub random_peers_answers: u64,
    /// `tonNode.capabilities` answers sent.
    pub capabilities_answers: u64,
    /// `overlay.getRandomPeers` queries this node sent (session handshake,
    /// keepalives, and channel resubscribes). Comparing it with
    /// [`ProtocolStats::membership_answers`] shows how many peers actually
    /// answer us.
    pub random_peers_queries_sent: u64,
    /// Answers that failed to leave the node (send errors).
    pub query_answers_failed: u64,
    /// `overlay.getRandomPeers` answers received from peers.
    pub membership_answers: u64,
    /// Received answers that listed this node among the members.
    pub membership_listed: u64,
    /// Distinct peers whose answers listed this node (a bounded dedup of
    /// [`ProtocolStats::membership_listed`]).
    pub membership_listing_peers: u64,
    /// Distinct peers that echoed a `recv_addr_list_version`, i.e. peers that
    /// hold a routable `adnl.addressList` for this node and can originate
    /// `overlay.ping` and broadcasts to it (ADNL-level, folded in from
    /// `tonutils_adnl::adnl::udp::address_storing_peers`).
    pub address_storing_peers: u64,
    /// Distinct peers whose `overlay.ping` this node answered.
    pub unique_pingers: u64,
    /// `dht.ping` queries answered.  pytoniq keeps its peer lists with
    /// `Node.ping`, which sends `dht.ping` wrapped in `overlay.query` every
    /// 60 s to nodes the peer keeps in its own peer list, so this counter is
    /// the observable signal of admission by pytoniq-style peers.
    pub dht_pings_received: u64,
    /// Custom overlay payloads handed to the scanner (broadcast candidates).
    pub custom_messages: u64,
    /// Distinct peers whose custom overlay payloads were handed to the
    /// scanner - the peers that push to this node after admitting it.
    pub pushers: u64,
    /// Custom overlay payloads suppressed by the raw-payload pre-dedup:
    /// byte-identical copies of a payload this node already accepted
    /// within the cache window, which never paid a TL parse or
    /// signature verification.
    pub payload_duplicates: u64,
    /// FEC symbol parts fanned out by this node's broadcasters.
    pub fec_parts_sent: u64,
    /// The subset of [`ProtocolStats::fec_parts_sent`] that carried a
    /// repair symbol (wire id at or above `symbols_count`).
    pub fec_repair_parts_sent: u64,
    /// FEC parts accepted into a reassembly - parameters validated and
    /// the symbol fed to a decoder - whether or not the broadcast
    /// decoded from it.
    pub fec_symbols_accepted: u64,
    /// The subset of [`ProtocolStats::fec_symbols_accepted`] that
    /// carried a repair symbol.
    pub fec_repair_symbols_received: u64,
    /// FEC broadcasts fully reconstructed, hash-verified, and unwrapped.
    pub fec_decoded: u64,
    /// FEC parts rejected on bounds: an unsupported FEC type, invalid
    /// reassembly parameters, or an excessive wire symbol id.
    pub fec_param_rejections: u64,
}

struct Counters {
    queries_received: AtomicU64,
    queries_wrapped: AtomicU64,
    queries_rejected: AtomicU64,
    queries_unhandled: AtomicU64,
    last_unhandled_query_id: AtomicU32,
    pongs_sent: AtomicU64,
    pings_received: AtomicU64,
    random_peers_answers: AtomicU64,
    capabilities_answers: AtomicU64,
    random_peers_queries_sent: AtomicU64,
    query_answers_failed: AtomicU64,
    membership_answers: AtomicU64,
    membership_listed: AtomicU64,
    membership_listing_peers: AtomicU64,
    unique_pingers: AtomicU64,
    dht_pings_received: AtomicU64,
    custom_messages: AtomicU64,
    pushers: AtomicU64,
    payload_duplicates: AtomicU64,
    fec_parts_sent: AtomicU64,
    fec_repair_parts_sent: AtomicU64,
    fec_symbols_accepted: AtomicU64,
    fec_repair_symbols_received: AtomicU64,
    fec_decoded: AtomicU64,
    fec_param_rejections: AtomicU64,
    pingers_seen: DistinctPeers,
    listing_peers_seen: DistinctPeers,
    pushers_seen: DistinctPeers,
}

/// Bounded set of distinct peer ids behind one counter.
struct DistinctPeers {
    seen: LazyLock<Mutex<HashSet<[u8; 32]>>>,
}

impl DistinctPeers {
    const fn new() -> Self {
        Self {
            seen: LazyLock::new(|| Mutex::new(HashSet::new())),
        }
    }

    /// Returns `true` when `peer` was not seen before.
    fn record(&self, peer: [u8; 32]) -> bool {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if seen.len() >= MAX_TRACKED_PEERS {
            return false;
        }
        seen.insert(peer)
    }
}

static COUNTERS: Counters = Counters {
    queries_received: AtomicU64::new(0),
    queries_wrapped: AtomicU64::new(0),
    queries_rejected: AtomicU64::new(0),
    queries_unhandled: AtomicU64::new(0),
    last_unhandled_query_id: AtomicU32::new(0),
    pongs_sent: AtomicU64::new(0),
    pings_received: AtomicU64::new(0),
    random_peers_answers: AtomicU64::new(0),
    capabilities_answers: AtomicU64::new(0),
    random_peers_queries_sent: AtomicU64::new(0),
    query_answers_failed: AtomicU64::new(0),
    membership_answers: AtomicU64::new(0),
    membership_listed: AtomicU64::new(0),
    membership_listing_peers: AtomicU64::new(0),
    unique_pingers: AtomicU64::new(0),
    dht_pings_received: AtomicU64::new(0),
    custom_messages: AtomicU64::new(0),
    pushers: AtomicU64::new(0),
    payload_duplicates: AtomicU64::new(0),
    fec_parts_sent: AtomicU64::new(0),
    fec_repair_parts_sent: AtomicU64::new(0),
    fec_symbols_accepted: AtomicU64::new(0),
    fec_repair_symbols_received: AtomicU64::new(0),
    fec_decoded: AtomicU64::new(0),
    fec_param_rejections: AtomicU64::new(0),
    pingers_seen: DistinctPeers::new(),
    listing_peers_seen: DistinctPeers::new(),
    pushers_seen: DistinctPeers::new(),
};

fn inc(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_query_received() {
    inc(&COUNTERS.queries_received);
}

pub(crate) fn record_query_wrapped() {
    inc(&COUNTERS.queries_wrapped);
}

pub(crate) fn record_query_rejected() {
    inc(&COUNTERS.queries_rejected);
}

pub(crate) fn record_query_unhandled(id: u32) {
    inc(&COUNTERS.queries_unhandled);
    COUNTERS
        .last_unhandled_query_id
        .store(id, Ordering::Relaxed);
}

pub(crate) fn record_pong_sent() {
    inc(&COUNTERS.pongs_sent);
}

pub(crate) fn record_random_peers_answer() {
    inc(&COUNTERS.random_peers_answers);
}

pub(crate) fn record_capabilities_answer() {
    inc(&COUNTERS.capabilities_answers);
}

pub(crate) fn record_random_peers_query_sent() {
    inc(&COUNTERS.random_peers_queries_sent);
}

pub(crate) fn record_query_answer_failed() {
    inc(&COUNTERS.query_answers_failed);
}

pub(crate) fn record_membership_answer(listed: bool, peer: [u8; 32]) {
    inc(&COUNTERS.membership_answers);
    if listed {
        inc(&COUNTERS.membership_listed);
        if COUNTERS.listing_peers_seen.record(peer) {
            inc(&COUNTERS.membership_listing_peers);
        }
    }
}

pub(crate) fn record_ping_from(peer: [u8; 32]) {
    inc(&COUNTERS.pings_received);
    if COUNTERS.pingers_seen.record(peer) {
        inc(&COUNTERS.unique_pingers);
    }
}

pub(crate) fn record_dht_ping() {
    inc(&COUNTERS.dht_pings_received);
}

pub(crate) fn record_custom_message_from(peer: [u8; 32]) {
    inc(&COUNTERS.custom_messages);
    if COUNTERS.pushers_seen.record(peer) {
        inc(&COUNTERS.pushers);
    }
}

pub(crate) fn record_payload_duplicate() {
    inc(&COUNTERS.payload_duplicates);
}

pub(crate) fn record_fec_part_sent(repair: bool) {
    inc(&COUNTERS.fec_parts_sent);
    if repair {
        inc(&COUNTERS.fec_repair_parts_sent);
    }
}

pub(crate) fn record_fec_symbol(repair: bool) {
    inc(&COUNTERS.fec_symbols_accepted);
    if repair {
        inc(&COUNTERS.fec_repair_symbols_received);
    }
}

pub(crate) fn record_fec_decoded() {
    inc(&COUNTERS.fec_decoded);
}

pub(crate) fn record_fec_param_rejection() {
    inc(&COUNTERS.fec_param_rejections);
}

/// Returns the current overlay protocol counters.
pub fn protocol_stats() -> ProtocolStats {
    ProtocolStats {
        queries_received: COUNTERS.queries_received.load(Ordering::Relaxed),
        queries_wrapped: COUNTERS.queries_wrapped.load(Ordering::Relaxed),
        queries_rejected: COUNTERS.queries_rejected.load(Ordering::Relaxed),
        queries_unhandled: COUNTERS.queries_unhandled.load(Ordering::Relaxed),
        last_unhandled_query_id: COUNTERS.last_unhandled_query_id.load(Ordering::Relaxed),
        pongs_sent: COUNTERS.pongs_sent.load(Ordering::Relaxed),
        pings_received: COUNTERS.pings_received.load(Ordering::Relaxed),
        random_peers_answers: COUNTERS.random_peers_answers.load(Ordering::Relaxed),
        capabilities_answers: COUNTERS.capabilities_answers.load(Ordering::Relaxed),
        random_peers_queries_sent: COUNTERS.random_peers_queries_sent.load(Ordering::Relaxed),
        query_answers_failed: COUNTERS.query_answers_failed.load(Ordering::Relaxed),
        membership_answers: COUNTERS.membership_answers.load(Ordering::Relaxed),
        membership_listed: COUNTERS.membership_listed.load(Ordering::Relaxed),
        membership_listing_peers: COUNTERS.membership_listing_peers.load(Ordering::Relaxed),
        address_storing_peers: tonutils_adnl::adnl::udp::address_storing_peers(),
        unique_pingers: COUNTERS.unique_pingers.load(Ordering::Relaxed),
        dht_pings_received: COUNTERS.dht_pings_received.load(Ordering::Relaxed),
        custom_messages: COUNTERS.custom_messages.load(Ordering::Relaxed),
        pushers: COUNTERS.pushers.load(Ordering::Relaxed),
        payload_duplicates: COUNTERS.payload_duplicates.load(Ordering::Relaxed),
        fec_parts_sent: COUNTERS.fec_parts_sent.load(Ordering::Relaxed),
        fec_repair_parts_sent: COUNTERS.fec_repair_parts_sent.load(Ordering::Relaxed),
        fec_symbols_accepted: COUNTERS.fec_symbols_accepted.load(Ordering::Relaxed),
        fec_repair_symbols_received: COUNTERS.fec_repair_symbols_received.load(Ordering::Relaxed),
        fec_decoded: COUNTERS.fec_decoded.load(Ordering::Relaxed),
        fec_param_rejections: COUNTERS.fec_param_rejections.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::{DistinctPeers, MAX_TRACKED_PEERS};

    #[test]
    fn distinct_peers_dedup_and_saturate() {
        let peers = DistinctPeers::new();
        assert!(peers.record([1; 32]), "a new peer must count");
        assert!(!peers.record([1; 32]), "the same peer must dedup");
        assert!(peers.record([2; 32]), "a second peer must count");

        let saturating = DistinctPeers::new();
        for index in 0..MAX_TRACKED_PEERS {
            let mut id = [0u8; 32];
            id[..8].copy_from_slice(&u64::try_from(index).expect("index fits").to_le_bytes());
            assert!(
                saturating.record(id),
                "peer {index} must count while below the cap"
            );
        }
        assert!(
            !saturating.record([255; 32]),
            "a saturated set must stop counting distinct peers"
        );
    }
}
