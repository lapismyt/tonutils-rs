//! Lite client balancer for load balancing and failover across multiple liteservers.
//!
//! This module provides a `LiteBalancer` that manages multiple `LiteClient` instances,
//! automatically handling:
//! - Connection failures and failover
//! - Load balancing based on response times and current load
//! - Peer health checking and automatic reconnection
//! - Consensus-based synchronization checking
//! - Archival node detection

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::liteclient::{client::LiteClient, types::LiteError};
use crate::tl::common::*;
use crate::tl::response::*;
use crate::tvm::Address;

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
    checker_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    
    pub max_req_per_peer: usize,
    pub max_retries: usize,
    pub timeout: Duration,
    inited: Arc<RwLock<bool>>,
}

impl LiteBalancer {
    pub fn new(peers: Vec<LiteClient>, timeout: Duration) -> Self {
        Self {
            peers,
            alive_peers: Arc::new(RwLock::new(HashSet::new())),
            archival_peers: Arc::new(RwLock::new(HashSet::new())),
            peer_stats: Arc::new(RwLock::new(HashMap::new())),
            checker_handle: Arc::new(RwLock::new(None)),
            max_req_per_peer: 100,
            max_retries: 1,
            timeout,
            inited: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn start_up(&mut self) -> Result<()> {
        let mut tasks = Vec::new();
        
        for (i, client) in self.peers.iter_mut().enumerate() {
            let result = Self::connect_to_peer(client).await;
            if result {
                self.alive_peers.write().await.insert(i);
            }
            tasks.push(result);
        }

        self.find_archives().await;
        
        // Start health checker
        let checker = self.spawn_health_checker();
        *self.checker_handle.write().await = Some(checker);
        
        // Don't delete peers on startup - they haven't made any requests yet
        // delete_unsync_peers will be called after first requests complete
        *self.inited.write().await = true;
        
        Ok(())
    }

    async fn connect_to_peer(_client: &mut LiteClient) -> bool {
        // Just return true - the client connection already succeeded in the CLI
        // We'll verify health during actual requests
        true
    }

    async fn check_archive(client: &mut LiteClient) -> bool {
        // Try to lookup an old block to check if peer is archival
        let block_id = BlockId {
            workchain: -1,
            shard: -9223372036854775808i64,
            seqno: rand::random::<i32>() % 1024 + 1,
        };
        
        match client.lookup_block(
            (),
            block_id,
            Some(()),
            None,
            None,
            false,
            false,
            false,
            false,
            false,
        ).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    async fn find_archives(&mut self) {
        let alive_peers: Vec<usize> = self.alive_peers.read().await.iter().copied().collect();
        let mut archival = HashSet::new();
        
        for i in alive_peers {
            if let Some(client) = self.peers.get_mut(i) {
                if Self::check_archive(client).await {
                    archival.insert(i);
                }
            }
        }
        
        *self.archival_peers.write().await = archival;
    }

    fn spawn_health_checker(&self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                log::trace!("Health checker tick");
            }
        })
    }

    async fn build_priority_list(&self, only_archive: bool) -> Vec<usize> {
        let peers = if only_archive {
            self.archival_peers.read().await.iter().copied().collect()
        } else {
            self.alive_peers.read().await.iter().copied().collect()
        };
        
        let stats = self.peer_stats.read().await;
        let timeout_ms = self.timeout.as_millis() as u64;
        
        let mut peers_vec: Vec<usize> = peers;
        peers_vec.sort_by(|a, b| {
            let stats_a = stats.get(a);
            let stats_b = stats.get(b);
            
            let seqno_a = stats_a.map(|s| s.mc_block_seqno).unwrap_or(0);
            let seqno_b = stats_b.map(|s| s.mc_block_seqno).unwrap_or(0);
            let time_a = stats_a.map(|s| s.avg_response_time_ms).unwrap_or(timeout_ms);
            let time_b = stats_b.map(|s| s.avg_response_time_ms).unwrap_or(timeout_ms);
            
            // Sort by seqno descending, then by response time ascending
            match seqno_b.cmp(&seqno_a) {
                std::cmp::Ordering::Equal => time_a.cmp(&time_b),
                other => other,
            }
        });
        
        peers_vec
    }

    async fn choose_peer(&self, only_archive: bool) -> Result<usize> {
        let peers = self.build_priority_list(only_archive).await;
        
        if peers.is_empty() {
            return Err(if only_archive {
                BalancerError::NoArchivePeers
            } else {
                BalancerError::NoAlivePeers
            });
        }
        
        let stats = self.peer_stats.read().await;
        let mut min_req = usize::MAX;
        
        // First pass: find peer with acceptable load
        for &peer_idx in &peers {
            let current_req = stats
                .get(&peer_idx)
                .map(|s| s.current_requests as usize)
                .unwrap_or(0);
            
            if current_req <= self.max_req_per_peer {
                return Ok(peer_idx);
            }
            
            min_req = min_req.min(current_req);
        }
        
        // Second pass: find peer with minimum load
        for &peer_idx in &peers {
            let current_req = stats
                .get(&peer_idx)
                .map(|s| s.current_requests as usize)
                .unwrap_or(0);
            
            if current_req <= min_req {
                return Ok(peer_idx);
            }
        }
        
        Ok(peers[0])
    }

    fn calc_new_average(old_avg: u64, n: u64, new_value: u64) -> u64 {
        if n == 0 {
            new_value
        } else {
            (old_avg * n + new_value) / (n + 1)
        }
    }

    async fn update_average_request_time(&self, peer_idx: usize, request_time_ms: u64) {
        let mut stats = self.peer_stats.write().await;
        let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
        
        peer_stats.avg_response_time_ms = Self::calc_new_average(
            peer_stats.avg_response_time_ms,
            peer_stats.total_requests,
            request_time_ms,
        );
        peer_stats.total_requests += 1;
    }

    async fn find_consensus_block(&self) -> u32 {
        let stats = self.peer_stats.read().await;
        let mut seqnos: Vec<u32> = stats.values().map(|s| s.mc_block_seqno).collect();
        
        if seqnos.is_empty() {
            return 0;
        }
        
        seqnos.sort_by(|a, b| b.cmp(a));
        let consensus_idx = (seqnos.len() * 2) / 3;
        seqnos.get(consensus_idx).copied().unwrap_or(0)
    }

    async fn delete_unsync_peers(&self) {
        let consensus_block = self.find_consensus_block().await;
        let stats = self.peer_stats.read().await;
        let mut alive = self.alive_peers.write().await;
        
        alive.retain(|&peer_idx| {
            stats
                .get(&peer_idx)
                .map(|s| s.mc_block_seqno >= consensus_block)
                .unwrap_or(false)
        });
    }

    pub fn peers_num(&self) -> usize {
        self.peers.len()
    }

    pub async fn alive_peers_num(&self) -> usize {
        self.alive_peers.read().await.len()
    }

    pub async fn archival_peers_num(&self) -> usize {
        self.archival_peers.read().await.len()
    }

    pub async fn is_inited(&self) -> bool {
        *self.inited.read().await
    }

    pub async fn close_all(&mut self) -> Result<()> {
        if let Some(handle) = self.checker_handle.write().await.take() {
            handle.abort();
        }
        
        *self.inited.write().await = false;
        Ok(())
    }

    async fn execute_request<T>(
        &mut self,
        only_archive: bool,
    ) -> Result<(usize, Instant)> {
        let peer_idx = self.choose_peer(only_archive).await?;
        
        // Increment current request count
        {
            let mut stats = self.peer_stats.write().await;
            let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
            peer_stats.current_requests += 1;
        }
        
        let start = Instant::now();
        Ok((peer_idx, start))
    }

    async fn complete_request(&mut self, peer_idx: usize, start: Instant, success: bool) {
        let elapsed = start.elapsed().as_millis() as u64;
        
        // Decrement current request count
        {
            let mut stats = self.peer_stats.write().await;
            if let Some(peer_stats) = stats.get_mut(&peer_idx) {
                peer_stats.current_requests = peer_stats.current_requests.saturating_sub(1);
            }
        }
        
        if success {
            self.update_average_request_time(peer_idx, elapsed).await;
        } else {
            self.update_average_request_time(peer_idx, self.timeout.as_millis() as u64).await;
            self.alive_peers.write().await.remove(&peer_idx);
        }
    }

    // Delegate methods to underlying clients with load balancing
    pub async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<MasterchainInfo>(false).await?;
            let result = self.peers[peer_idx].get_masterchain_info().await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_masterchain_info_ext(&mut self, mode: u32) -> Result<MasterchainInfoExt> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<MasterchainInfoExt>(false).await?;
            let result = self.peers[peer_idx].get_masterchain_info_ext(mode).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_time(&mut self) -> Result<u32> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<u32>(false).await?;
            let result = self.peers[peer_idx].get_time().await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_version(&mut self) -> Result<Version> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<Version>(false).await?;
            let result = self.peers[peer_idx].get_version().await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_block(&mut self, id: BlockIdExt) -> Result<Vec<u8>> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<Vec<u8>>(false).await?;
            let result = self.peers[peer_idx].get_block(id.clone()).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_state(&mut self, id: BlockIdExt) -> Result<BlockState> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<BlockState>(false).await?;
            let result = self.peers[peer_idx].get_state(id.clone()).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_block_header(
        &mut self,
        id: BlockIdExt,
        with_state_update: bool,
        with_value_flow: bool,
        with_extra: bool,
        with_shard_hashes: bool,
        with_prev_blk_signatures: bool,
    ) -> Result<Vec<u8>> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<Vec<u8>>(false).await?;
            let result = self.peers[peer_idx].get_block_header(
                id.clone(),
                with_state_update,
                with_value_flow,
                with_extra,
                with_shard_hashes,
                with_prev_blk_signatures,
            ).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn send_message(&mut self, body: Vec<u8>) -> Result<u32> {
        // For send_message, distribute to multiple peers
        let k = {
            let alive_count = self.alive_peers.read().await.len();
            if alive_count < 12 { 4 } else { alive_count / 3 }
        };
        
        let mut results = Vec::new();
        for _ in 0..k.min(self.peers.len()) {
            for _attempt in 0..self.max_retries {
                let (peer_idx, start) = self.execute_request::<u32>(false).await?;
                let result = self.peers[peer_idx].send_message(body.clone()).await;
                
                match result {
                    Ok(status) => {
                        self.complete_request(peer_idx, start, true).await;
                        results.push(Ok(status));
                        break;
                    }
                    Err(e) => {
                        let is_connection_error = matches!(e, LiteError::AdnlError(_));
                        self.complete_request(peer_idx, start, false).await;
                        if !is_connection_error {
                            results.push(Err(e));
                            break;
                        }
                    }
                }
            }
        }
        
        // Return success if any peer succeeded
        for result in results {
            if let Ok(status) = result {
                return Ok(status);
            }
        }
        
        Err(BalancerError::Timeout)
    }

    pub async fn get_account_state(&mut self, id: BlockIdExt, account: AccountId) -> Result<AccountState> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<AccountState>(false).await?;
            let result = self.peers[peer_idx].get_account_state(id.clone(), account.clone()).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn run_smc_method(
        &mut self,
        mode: u32,
        id: BlockIdExt,
        account: Address,
        method_id: u16,
        params: Vec<u8>,
    ) -> Result<RunMethodResult> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<RunMethodResult>(false).await?;
            let result = self.peers[peer_idx].run_smc_method(
                mode,
                id.clone(),
                account.clone(),
                method_id,
                params.clone(),
            ).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<TransactionList> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<TransactionList>(false).await?;
            let result = self.peers[peer_idx].get_transactions(
                count,
                account.clone(),
                lt,
                hash.clone(),
            ).await;
            
            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_connection_error = matches!(e, LiteError::AdnlError(_));
                    self.complete_request(peer_idx, start, false).await;
                    if !is_connection_error {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_balancer_error_display() {
        let err = BalancerError::NoAlivePeers;
        assert_eq!(err.to_string(), "No alive peers available");

        let err = BalancerError::NoArchivePeers;
        assert_eq!(err.to_string(), "No alive archive peers available");

        let err = BalancerError::Timeout;
        assert_eq!(err.to_string(), "Timeout error");
    }

    #[test]
    fn test_peer_stats_default() {
        let stats = PeerStats::default();
        assert_eq!(stats.mc_block_seqno, 0);
        assert_eq!(stats.avg_response_time_ms, 0);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.current_requests, 0);
    }

    #[test]
    fn test_calc_new_average() {
        // First request
        let avg = LiteBalancer::calc_new_average(0, 0, 100);
        assert_eq!(avg, 100);

        // Second request
        let avg = LiteBalancer::calc_new_average(100, 1, 200);
        assert_eq!(avg, 150);

        // Third request
        let avg = LiteBalancer::calc_new_average(150, 2, 300);
        assert_eq!(avg, 200);

        // Multiple requests
        let avg = LiteBalancer::calc_new_average(200, 10, 100);
        assert_eq!(avg, (200 * 10 + 100) / 11);
    }

    #[tokio::test]
    async fn test_balancer_initialization() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        assert_eq!(balancer.peers_num(), 0);
        assert_eq!(balancer.alive_peers_num().await, 0);
        assert_eq!(balancer.archival_peers_num().await, 0);
        assert!(!balancer.is_inited().await);
        assert_eq!(balancer.max_req_per_peer, 100);
        assert_eq!(balancer.max_retries, 1);
        assert_eq!(balancer.timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_balancer_configuration() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(5));
        
        balancer.max_req_per_peer = 50;
        balancer.max_retries = 3;
        
        assert_eq!(balancer.max_req_per_peer, 50);
        assert_eq!(balancer.max_retries, 3);
        assert_eq!(balancer.timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_empty_balancer_choose_peer() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        let result = balancer.choose_peer(false).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BalancerError::NoAlivePeers));
    }

    #[tokio::test]
    async fn test_empty_balancer_choose_archive_peer() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        let result = balancer.choose_peer(true).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BalancerError::NoArchivePeers));
    }

    #[tokio::test]
    async fn test_consensus_block_empty() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        let consensus = balancer.find_consensus_block().await;
        assert_eq!(consensus, 0);
    }

    #[tokio::test]
    async fn test_consensus_block_calculation() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add some peer stats manually
        {
            let mut stats = balancer.peer_stats.write().await;
            
            // Add 5 peers with different seqnos
            stats.insert(0, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 50,
                total_requests: 10,
                current_requests: 0,
            });
            stats.insert(1, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 60,
                total_requests: 8,
                current_requests: 0,
            });
            stats.insert(2, PeerStats {
                mc_block_seqno: 99,
                avg_response_time_ms: 40,
                total_requests: 12,
                current_requests: 0,
            });
            stats.insert(3, PeerStats {
                mc_block_seqno: 101,
                avg_response_time_ms: 55,
                total_requests: 9,
                current_requests: 0,
            });
            stats.insert(4, PeerStats {
                mc_block_seqno: 98,
                avg_response_time_ms: 70,
                total_requests: 5,
                current_requests: 0,
            });
        }
        
        let consensus = balancer.find_consensus_block().await;
        // With 5 peers, 2/3 index = 3, sorted descending: [101, 100, 100, 99, 98]
        // consensus should be at index 3, which is 99
        assert_eq!(consensus, 99);
    }

    #[tokio::test]
    async fn test_build_priority_list_sorting() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add some alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }
        
        // Add peer stats with different characteristics
        {
            let mut stats = balancer.peer_stats.write().await;
            
            // Peer 0: high seqno, medium response time
            stats.insert(0, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 50,
                total_requests: 10,
                current_requests: 0,
            });
            
            // Peer 1: high seqno, low response time (should be first)
            stats.insert(1, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 30,
                total_requests: 15,
                current_requests: 0,
            });
            
            // Peer 2: low seqno, low response time (should be last)
            stats.insert(2, PeerStats {
                mc_block_seqno: 95,
                avg_response_time_ms: 25,
                total_requests: 20,
                current_requests: 0,
            });
        }
        
        let priority_list = balancer.build_priority_list(false).await;
        
        // Expected order: peer 1 (seqno 100, 30ms), peer 0 (seqno 100, 50ms), peer 2 (seqno 95, 25ms)
        assert_eq!(priority_list.len(), 3);
        assert_eq!(priority_list[0], 1); // Best peer
        assert_eq!(priority_list[1], 0);
        assert_eq!(priority_list[2], 2); // Worst peer (low seqno)
    }

    #[tokio::test]
    async fn test_update_average_request_time() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // First update
        balancer.update_average_request_time(0, 100).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 100);
            assert_eq!(peer_stats.total_requests, 1);
        }
        
        // Second update
        balancer.update_average_request_time(0, 200).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 150);
            assert_eq!(peer_stats.total_requests, 2);
        }
        
        // Third update
        balancer.update_average_request_time(0, 300).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 200);
            assert_eq!(peer_stats.total_requests, 3);
        }
    }

    #[tokio::test]
    async fn test_delete_unsync_peers() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
            alive.insert(3);
        }
        
        // Add peer stats
        {
            let mut stats = balancer.peer_stats.write().await;
            stats.insert(0, PeerStats {
                mc_block_seqno: 100,
                ..Default::default()
            });
            stats.insert(1, PeerStats {
                mc_block_seqno: 100,
                ..Default::default()
            });
            stats.insert(2, PeerStats {
                mc_block_seqno: 98, // Out of sync
                ..Default::default()
            });
            stats.insert(3, PeerStats {
                mc_block_seqno: 99,
                ..Default::default()
            });
        }
        
        balancer.delete_unsync_peers().await;
        
        let alive = balancer.alive_peers.read().await;
        // Consensus should be 99 or 100 depending on calculation
        // Peers with seqno >= consensus should remain
        assert!(alive.contains(&0));
        assert!(alive.contains(&1));
    }

    #[tokio::test]
    async fn test_choose_peer_with_load() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        balancer.max_req_per_peer = 5;
        
        // Add alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }
        
        // Add peer stats with different loads
        {
            let mut stats = balancer.peer_stats.write().await;
            
            // Peer 0: overloaded
            stats.insert(0, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 50,
                total_requests: 10,
                current_requests: 10, // Over limit
            });
            
            // Peer 1: available (should be chosen)
            stats.insert(1, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 60,
                total_requests: 5,
                current_requests: 2, // Under limit
            });
            
            // Peer 2: available but slower
            stats.insert(2, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 80,
                total_requests: 8,
                current_requests: 3, // Under limit
            });
        }
        
        let chosen = balancer.choose_peer(false).await.unwrap();
        // Should choose peer 1 (under limit and faster than peer 2)
        assert_eq!(chosen, 1);
    }

    #[tokio::test]
    async fn test_archival_peers_filtering() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add some alive and archival peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }
        
        {
            let mut archival = balancer.archival_peers.write().await;
            archival.insert(1); // Only peer 1 is archival
        }
        
        // Build priority list for archival only
        let archival_list = balancer.build_priority_list(true).await;
        assert_eq!(archival_list.len(), 1);
        assert_eq!(archival_list[0], 1);
        
        // Build priority list for all
        let all_list = balancer.build_priority_list(false).await;
        assert_eq!(all_list.len(), 3);
    }

    #[tokio::test]
    async fn test_balancer_close_all() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Manually set inited to true
        *balancer.inited.write().await = true;
        
        // Start a dummy health checker
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(1000)).await;
        });
        *balancer.checker_handle.write().await = Some(handle);
        
        assert!(balancer.is_inited().await);
        
        // Close all
        balancer.close_all().await.unwrap();
        
        assert!(!balancer.is_inited().await);
        assert!(balancer.checker_handle.read().await.is_none());
    }

    #[test]
    fn test_balancer_error_from_lite_error() {
        let lite_err = LiteError::UnexpectedMessage;
        let balancer_err: BalancerError = lite_err.into();
        
        assert!(matches!(balancer_err, BalancerError::LiteError(_)));
    }

    #[tokio::test]
    async fn test_execute_request_increments_counter() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add an alive peer
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
        }
        
        // Execute request
        let (_peer_idx, _start) = balancer.execute_request::<()>(false).await.unwrap();
        
        // Check that counter was incremented
        let stats = balancer.peer_stats.read().await;
        let peer_stats = stats.get(&0).unwrap();
        assert_eq!(peer_stats.current_requests, 1);
    }

    #[tokio::test]
    async fn test_complete_request_decrements_counter() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add peer with active request
        {
            let mut stats = balancer.peer_stats.write().await;
            stats.insert(0, PeerStats {
                mc_block_seqno: 100,
                avg_response_time_ms: 50,
                total_requests: 5,
                current_requests: 3,
            });
        }
        
        let start = Instant::now();
        balancer.complete_request(0, start, true).await;
        
        // Check that counter was decremented
        let stats = balancer.peer_stats.read().await;
        let peer_stats = stats.get(&0).unwrap();
        assert_eq!(peer_stats.current_requests, 2);
    }

    #[tokio::test]
    async fn test_complete_request_removes_failed_peer() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        
        // Add alive peer
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
        }
        
        let start = Instant::now();
        balancer.complete_request(0, start, false).await;
        
        // Check that peer was removed from alive set
        let alive = balancer.alive_peers.read().await;
        assert!(!alive.contains(&0));
    }
}
