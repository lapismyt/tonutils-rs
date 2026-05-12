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

type Result<T> = std::result::Result<T, BalancerError>;

macro_rules! balanced_call {
    ($self:ident, $response:ty, $only_archive:expr, |$client:ident| $call:expr) => {{
        for _attempt in 0..$self.max_retries {
            let (peer_idx, start) = $self.execute_request::<$response>($only_archive).await?;
            let result = {
                let $client = &mut $self.peers[peer_idx];
                $call.await
            };

            match result {
                Ok(response) => {
                    $self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    $self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Healthy,
    Suspect,
    Dead,
    Recovering,
}

struct PeerStats {
    mc_block_seqno: u32,
    avg_response_time_ms: u64,
    total_requests: u64,
    current_requests: u64,
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
    peers: Vec<LiteClient>,
    alive_peers: Arc<RwLock<HashSet<usize>>>,
    archival_peers: Arc<RwLock<HashSet<usize>>>,
    peer_stats: Arc<RwLock<HashMap<usize, PeerStats>>>,
    peer_states: Arc<RwLock<HashMap<usize, PeerState>>>,
    checker_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    global_rate_limiter: Option<RateLimiter>,

    pub max_req_per_peer: usize,
    pub max_retries: usize,
    pub timeout: Duration,
    inited: Arc<RwLock<bool>>,
}

