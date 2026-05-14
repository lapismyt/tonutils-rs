use super::*;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::liteclient::{
    boc::{
        DecodedAccountState, DecodedAllShardsInfo, DecodedBlockData, DecodedBlockHeader,
        DecodedBlockTransactionsExt, DecodedConfigInfo, DecodedLibrariesWithProof,
        DecodedShardInfo, SimpleAccount,
    },
    client::LiteClient,
    rate_limit::{RateLimiter, RequestRateLimit},
    types::LiteError,
};
use crate::tl::common::*;
use crate::tl::response::*;
use crate::tvm::{Address, TvmStack, TvmStackEntry};

#[derive(Debug, thiserror::Error)]
pub enum BalancerError {
    #[error("No alive peers available")]
    NoAlivePeers,
    #[error("No alive archive peers available")]
    NoArchivePeers,
    #[error("Use start_up() instead of connect()")]
    UseStartUp,
    #[error("Use close_all() instead of close()")]
    UseCloseAll,
    #[error("Lite client error: {0}")]
    LiteError(#[from] LiteError),
    #[error("Timeout error")]
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Healthy,
    Suspect,
    Dead,
    Recovering,
}

pub(super) struct PeerStats {
    pub(super) mc_block_seqno: u32,
    pub(super) avg_response_time_ms: u64,
    pub(super) total_requests: u64,
    pub(super) current_requests: u64,
}

impl Default for PeerStats {
    fn default() -> Self {
        Self {
            mc_block_seqno: 0,
            avg_response_time_ms: 0,
            total_requests: 0,
            current_requests: 0,
        }
    }
}

pub struct LiteBalancer {
    pub(super) peers: Vec<LiteClient>,
    pub(super) alive_peers: Arc<RwLock<HashSet<usize>>>,
    pub(super) archival_peers: Arc<RwLock<HashSet<usize>>>,
    pub(super) peer_stats: Arc<RwLock<HashMap<usize, PeerStats>>>,
    pub(super) peer_states: Arc<RwLock<HashMap<usize, PeerState>>>,
    pub(super) checker_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    pub(super) global_rate_limiter: Option<RateLimiter>,

    pub max_req_per_peer: usize,
    pub max_retries: usize,
    pub timeout: Duration,
    pub(super) inited: Arc<RwLock<bool>>,
}
