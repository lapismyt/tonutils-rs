//! Lite client balancer for load balancing and failover across multiple liteservers.
//!
//! This module provides a `LiteBalancer` that manages multiple `LiteClient` instances,
//! automatically handling:
//! - Connection failures and failover
//! - Load balancing based on response times and current load
//! - Peer health checking and automatic reconnection
//! - Best-effort synchronization filtering based on observed masterchain seqnos
//! - Archival node detection

pub(super) use crate::liteclient::{
    boc::{
        DecodedAccountState, DecodedAllShardsInfo, DecodedBlockData, DecodedBlockHeader,
        DecodedBlockTransactionsExt, DecodedConfigInfo, DecodedLibrariesWithProof,
        DecodedShardInfo, SimpleAccount,
    },
    client::LiteClient,
    rate_limit::RateLimiter,
    rate_limit::RequestRateLimit,
    types::LiteError,
};
pub(super) use crate::tl::common::*;
pub(super) use crate::tl::response::*;
pub(super) use crate::tvm::{Address, TvmStack, TvmStackEntry};
pub(super) use std::collections::{HashMap, HashSet};
pub(super) use std::sync::Arc;
pub(super) use std::time::{Duration, Instant};
pub(super) use tokio::sync::RwLock;
pub(super) use tokio::task::JoinHandle;
pub(super) type Result<T> = std::result::Result<T, BalancerError>;

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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    $self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }};
}

mod archive;
mod execute;
mod helpers;
#[cfg(test)]
mod reliability_tests;
mod selection;
mod state;
#[cfg(test)]
mod tests;
mod types;

use types::*;

pub use types::*;
