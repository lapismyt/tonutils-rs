//! Age-aware sharded external-message deduplication.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

use super::MempoolConfig;

pub(crate) struct DedupShard {
    entries: HashMap<[u8; 32], Instant>,
    last_purge: Instant,
}

impl DedupShard {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_purge: Instant::now(),
        }
    }

    fn purge(&mut self, now: Instant, ttl: Duration) -> u64 {
        if now.duration_since(self.last_purge) < ttl {
            return 0;
        }
        self.last_purge = now;
        let before = self.entries.len();
        self.entries
            .retain(|_, seen| now.duration_since(*seen) < ttl);
        (before - self.entries.len()) as u64
    }

    fn duplicate(&mut self, hash: &[u8; 32], now: Instant, ttl: Duration) -> bool {
        let Some(seen) = self.entries.get(hash).copied() else {
            return false;
        };
        if now.duration_since(seen) < ttl {
            return true;
        }
        self.entries.remove(hash);
        false
    }
}

pub(crate) struct Dedup {
    config: MempoolConfig,
    shards: Arc<Vec<Mutex<DedupShard>>>,
    capacity: Mutex<()>,
    entries: AtomicU64,
    duplicates: Arc<AtomicU64>,
}

impl Dedup {
    pub(crate) fn new(config: MempoolConfig, duplicates: Arc<AtomicU64>) -> Self {
        let shards = (0..config.dedup_shards)
            .map(|_| Mutex::new(DedupShard::new()))
            .collect();
        Self {
            config,
            shards: Arc::new(shards),
            capacity: Mutex::new(()),
            entries: AtomicU64::new(0),
            duplicates,
        }
    }

    pub(crate) async fn accept(&self, hash: [u8; 32]) -> bool {
        let index = self.shard_index(&hash);
        let now = Instant::now();
        let mut shard = self.shards[index].lock().await;
        if shard.duplicate(&hash, now, self.config.dedup_ttl) {
            self.duplicates.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        drop(shard);

        let _capacity_guard = self.capacity.lock().await;
        let mut shard = self.shards[index].lock().await;
        let expired = shard.purge(now, self.config.dedup_ttl);
        if expired > 0 {
            self.entries.fetch_sub(expired, Ordering::Relaxed);
        }
        if shard.duplicate(&hash, now, self.config.dedup_ttl) {
            self.duplicates.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if self.entries.load(Ordering::Relaxed) >= self.config.max_dedup_entries as u64 {
            drop(shard);
            self.evict_oldest().await;
            shard = self.shards[index].lock().await;
            if shard.duplicate(&hash, now, self.config.dedup_ttl) {
                self.duplicates.fetch_add(1, Ordering::Relaxed);
                return false;
            }
        }
        if shard.entries.insert(hash, now).is_none() {
            self.entries.fetch_add(1, Ordering::Relaxed);
        }
        true
    }

    fn shard_index(&self, hash: &[u8; 32]) -> usize {
        let value = u32::from_ne_bytes([hash[0], hash[1], hash[2], hash[3]]) as usize;
        value % self.shards.len()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> u64 {
        self.entries.load(Ordering::Relaxed)
    }

    async fn evict_oldest(&self) {
        let mut candidates = Vec::new();
        for (index, shard) in self.shards.iter().enumerate() {
            let shard = shard.lock().await;
            candidates.extend(
                shard
                    .entries
                    .iter()
                    .map(|(hash, seen)| (*hash, index, *seen)),
            );
        }
        let evict_count = (self.config.max_dedup_entries / 10).max(1);
        candidates.sort_by_key(|(_, _, seen)| *seen);
        let mut removed = 0u64;
        for (hash, index, _) in candidates.into_iter().take(evict_count) {
            if self.shards[index]
                .lock()
                .await
                .entries
                .remove(&hash)
                .is_some()
            {
                removed += 1;
            }
        }
        if removed > 0 {
            self.entries.fetch_sub(removed, Ordering::Relaxed);
        }
    }
}
