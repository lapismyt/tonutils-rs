//! Shared raw-payload pre-dedup for the overlay receive paths.
//!
//! Relayed copies of one broadcast are byte-identical - relays forward
//! the origin's signed object verbatim - so the first sighting of a
//! payload hash can suppress the TL parse and ed25519 verification
//! every later copy would otherwise pay before the mempool's dedup
//! shards get a chance to reject it.  Upstream deduplicates early the
//! same way: `BroadcastFecPart::run` rejects duplicate part ids
//! before `run_checks`, and `BroadcastSimple` caches a broadcast id
//! with its checked state.
//!
//! The cache is shared by every session of one overlay client (flood copies
//! arrive from different peers), bounded, and time-expiring, so a
//! hash can never suppress a payload forever.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

/// Payload hash to its first-sighting timestamp and insertion sequence.
type Seen = HashMap<[u8; 32], (Instant, u64)>;

/// Bounded, time-expiring cache of recently seen raw payload hashes.
///
/// Entries carry a monotonic insertion sequence next to their
/// timestamp so eviction stays deterministic even when two inserts
/// share an `Instant` tick.
#[derive(Clone)]
pub struct PacketPreDedup {
    seen: Arc<Mutex<Seen>>,
    next_sequence: Arc<AtomicU64>,
    ttl: Duration,
    capacity: usize,
}

impl PacketPreDedup {
    /// Creates a cache that keeps a hash for `ttl` and holds at most
    /// `capacity` hashes, evicting expired entries first and the
    /// oldest survivor when a live flood still exceeds the bound.
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            seen: Arc::new(Mutex::new(HashMap::with_capacity(capacity))),
            next_sequence: Arc::new(AtomicU64::new(0)),
            ttl,
            capacity: capacity.max(1),
        }
    }

    /// Returns `true` when `payload` already appeared within the TTL,
    /// recording it otherwise.  Byte-identical copies are exactly the
    /// flood this cache exists to collapse: the first copy runs the
    /// full verify-and-ingest path, every later copy is dropped here
    /// for the cost of one hash.
    pub fn is_duplicate(&self, payload: &[u8]) -> bool {
        let hash: [u8; 32] = Sha256::digest(payload).into();
        let now = Instant::now();
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((last_seen, _)) = seen.get(&hash)
            && now.duration_since(*last_seen) < self.ttl
        {
            return true;
        }
        if seen.len() >= self.capacity {
            seen.retain(|_, (last_seen, _)| now.duration_since(*last_seen) < self.ttl);
            if seen.len() >= self.capacity
                && let Some(oldest) = seen
                    .iter()
                    .min_by_key(|(_, (last_seen, sequence))| (*last_seen, *sequence))
                    .map(|(hash, _)| *hash)
            {
                seen.remove(&oldest);
            }
        }
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        seen.insert(hash, (now, sequence));
        false
    }
}

impl Default for PacketPreDedup {
    fn default() -> Self {
        // The TTL mirrors `MempoolConfig::dedup_ttl`'s default: a
        // pre-dedup entry must not outlive - and thereby wrongly
        // suppress - the dedup shard entry the first copy recorded.
        Self::new(Duration::from_secs(300), 4096)
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, PacketPreDedup};

    #[test]
    fn first_payload_passes_and_copies_are_suppressed() {
        let dedup = PacketPreDedup::new(Duration::from_secs(60), 16);
        assert!(
            !dedup.is_duplicate(b"broadcast one"),
            "first sighting passes"
        );
        assert!(
            dedup.is_duplicate(b"broadcast one"),
            "the byte-identical copy is suppressed"
        );
        assert!(
            !dedup.is_duplicate(b"broadcast two"),
            "another payload is independent"
        );
    }

    #[test]
    fn zero_ttl_forgets_everything_immediately() {
        let dedup = PacketPreDedup::new(Duration::ZERO, 16);
        assert!(!dedup.is_duplicate(b"payload"));
        assert!(
            !dedup.is_duplicate(b"payload"),
            "an expired entry must not suppress"
        );
    }

    #[test]
    fn capacity_evicts_the_oldest_live_entry() {
        let dedup = PacketPreDedup::new(Duration::from_secs(600), 2);
        assert!(!dedup.is_duplicate(b"one"));
        assert!(!dedup.is_duplicate(b"two"));
        // The third payload pushes past the bound and evicts the
        // oldest entry ("one"); the recent ones stay suppressed.  A
        // hit returns before any insertion, so it cannot evict, and
        // the miss check runs last for the same reason.
        assert!(!dedup.is_duplicate(b"three"));
        assert!(dedup.is_duplicate(b"two"), "recent entry stays cached");
        assert!(dedup.is_duplicate(b"three"), "newest entry is cached");
        assert!(!dedup.is_duplicate(b"one"), "evicted entry passes again");
    }

    #[test]
    fn shared_clones_suppress_across_sessions() {
        let sender = PacketPreDedup::new(Duration::from_secs(60), 16);
        let receiver = sender.clone();
        assert!(!sender.is_duplicate(b"flood"));
        assert!(
            receiver.is_duplicate(b"flood"),
            "a clone shares the cache, as sibling sessions do"
        );
    }
}
