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
//!   member and therefore broadcast to it.
//!
//! All getters return a point-in-time [`ProtocolStats`] snapshot.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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
    /// `overlay.getRandomPeers` answers sent.
    pub random_peers_answers: u64,
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
    /// Custom overlay payloads handed to the scanner (broadcast candidates).
    pub custom_messages: u64,
}

struct Counters {
    queries_received: AtomicU64,
    queries_wrapped: AtomicU64,
    queries_rejected: AtomicU64,
    queries_unhandled: AtomicU64,
    last_unhandled_query_id: AtomicU32,
    pongs_sent: AtomicU64,
    random_peers_answers: AtomicU64,
    random_peers_queries_sent: AtomicU64,
    query_answers_failed: AtomicU64,
    membership_answers: AtomicU64,
    membership_listed: AtomicU64,
    custom_messages: AtomicU64,
}

static COUNTERS: Counters = Counters {
    queries_received: AtomicU64::new(0),
    queries_wrapped: AtomicU64::new(0),
    queries_rejected: AtomicU64::new(0),
    queries_unhandled: AtomicU64::new(0),
    last_unhandled_query_id: AtomicU32::new(0),
    pongs_sent: AtomicU64::new(0),
    random_peers_answers: AtomicU64::new(0),
    random_peers_queries_sent: AtomicU64::new(0),
    query_answers_failed: AtomicU64::new(0),
    membership_answers: AtomicU64::new(0),
    membership_listed: AtomicU64::new(0),
    custom_messages: AtomicU64::new(0),
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

pub(crate) fn record_random_peers_query_sent() {
    inc(&COUNTERS.random_peers_queries_sent);
}

pub(crate) fn record_query_answer_failed() {
    inc(&COUNTERS.query_answers_failed);
}

pub(crate) fn record_membership_answer(listed: bool) {
    inc(&COUNTERS.membership_answers);
    if listed {
        inc(&COUNTERS.membership_listed);
    }
}

pub(crate) fn record_custom_message() {
    inc(&COUNTERS.custom_messages);
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
        random_peers_answers: COUNTERS.random_peers_answers.load(Ordering::Relaxed),
        random_peers_queries_sent: COUNTERS.random_peers_queries_sent.load(Ordering::Relaxed),
        query_answers_failed: COUNTERS.query_answers_failed.load(Ordering::Relaxed),
        membership_answers: COUNTERS.membership_answers.load(Ordering::Relaxed),
        membership_listed: COUNTERS.membership_listed.load(Ordering::Relaxed),
        custom_messages: COUNTERS.custom_messages.load(Ordering::Relaxed),
    }
}
