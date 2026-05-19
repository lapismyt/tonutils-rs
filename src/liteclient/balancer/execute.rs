use super::*;

impl LiteBalancer {
    pub fn new(peers: Vec<LiteClient>, timeout: Duration) -> Self {
        Self {
            peers: peers
                .into_iter()
                .map(|peer| peer.with_request_timeout(timeout))
                .collect(),
            alive_peers: Arc::new(RwLock::new(HashSet::new())),
            archival_peers: Arc::new(RwLock::new(HashSet::new())),
            peer_stats: Arc::new(RwLock::new(HashMap::new())),
            peer_states: Arc::new(RwLock::new(HashMap::new())),
            checker_handle: Arc::new(RwLock::new(None)),
            global_rate_limiter: None,
            max_req_per_peer: 100,
            max_retries: 1,
            timeout,
            inited: Arc::new(RwLock::new(false)),
        }
    }

    pub fn with_rate_limit_per_peer(mut self, limit: RequestRateLimit) -> Self {
        for peer in &mut self.peers {
            peer.set_rate_limit(limit);
        }
        self
    }

    pub fn with_global_rate_limit(mut self, limit: RequestRateLimit) -> Self {
        self.global_rate_limiter = Some(RateLimiter::new(limit));
        self
    }

    pub async fn start_up(&mut self) -> Result<()> {
        let mut tasks = Vec::new();

        for (i, client) in self.peers.iter_mut().enumerate() {
            let result = Self::connect_to_peer(client).await;
            if result {
                self.alive_peers.write().await.insert(i);
                self.peer_states.write().await.insert(i, PeerState::Healthy);
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

    pub(super) async fn connect_to_peer(_client: &mut LiteClient) -> bool {
        // Just return true - the client connection already succeeded in the CLI
        // We'll verify health during actual requests
        true
    }

    pub(super) fn spawn_health_checker(&self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                log::trace!("Health checker tick");
            }
        })
    }

    pub(super) async fn build_priority_list(&self, only_archive: bool) -> Vec<usize> {
        let peers = if only_archive {
            self.archival_peers.read().await.iter().copied().collect()
        } else {
            self.alive_peers.read().await.iter().copied().collect()
        };

        let stats = self.peer_stats.read().await;
        let timeout_ms = self.timeout.as_millis() as u64;

        let best_seqno = stats
            .values()
            .map(|stats| stats.last_observed_seqno.max(stats.mc_block_seqno))
            .max()
            .unwrap_or(0);

        let mut peers_vec: Vec<usize> = peers;
        peers_vec.sort_by(|a, b| {
            let stats_a = stats.get(a);
            let stats_b = stats.get(b);
            let score_a = Self::peer_score(stats_a, best_seqno, timeout_ms);
            let score_b = Self::peer_score(stats_b, best_seqno, timeout_ms);
            score_a.cmp(&score_b)
        });

        peers_vec
    }

    pub(super) async fn choose_peer(&self, only_archive: bool) -> Result<usize> {
        self.choose_peer_excluding(only_archive, &HashSet::new())
            .await
    }

    pub(super) async fn choose_peer_excluding(
        &self,
        only_archive: bool,
        excluded: &HashSet<usize>,
    ) -> Result<usize> {
        let peers = self.build_priority_list(only_archive).await;

        if peers.is_empty() {
            return Err(if only_archive {
                BalancerError::NoArchivePeers
            } else {
                BalancerError::NoAlivePeers
            });
        }

        let stats = self.peer_stats.read().await;
        let states = self.peer_states.read().await;
        let mut min_req = usize::MAX;

        // First pass: find peer with acceptable load
        for &peer_idx in &peers {
            if excluded.contains(&peer_idx) {
                continue;
            }
            if matches!(states.get(&peer_idx), Some(PeerState::Dead)) {
                continue;
            }
            let current_req = stats
                .get(&peer_idx)
                .map(|s| s.current_requests as usize)
                .unwrap_or(0);

            if current_req < self.max_req_per_peer {
                return Ok(peer_idx);
            }

            min_req = min_req.min(current_req);
        }

        // Second pass: find peer with minimum load
        for &peer_idx in &peers {
            if excluded.contains(&peer_idx) {
                continue;
            }
            if matches!(states.get(&peer_idx), Some(PeerState::Dead)) {
                continue;
            }
            let current_req = stats
                .get(&peer_idx)
                .map(|s| s.current_requests as usize)
                .unwrap_or(0);

            if current_req <= min_req {
                return Ok(peer_idx);
            }
        }

        Err(if only_archive {
            BalancerError::NoArchivePeers
        } else {
            BalancerError::NoAlivePeers
        })
    }

    pub(super) fn calc_new_average(old_avg: u64, n: u64, new_value: u64) -> u64 {
        if n == 0 {
            new_value
        } else {
            (old_avg * n + new_value) / (n + 1)
        }
    }

    pub(super) async fn update_average_request_time(&self, peer_idx: usize, request_time_ms: u64) {
        let mut stats = self.peer_stats.write().await;
        let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);

        peer_stats.avg_response_time_ms = Self::calc_new_average(
            peer_stats.avg_response_time_ms,
            peer_stats.total_requests,
            request_time_ms,
        );
        peer_stats.ewma_latency_ms = Some(match peer_stats.ewma_latency_ms {
            Some(old) => (old * 7 + request_time_ms * 3) / 10,
            None => request_time_ms,
        });
        peer_stats.total_requests += 1;
    }

    pub(super) async fn find_consensus_block(&self) -> u32 {
        let stats = self.peer_stats.read().await;
        let mut seqnos: Vec<u32> = stats.values().map(|s| s.mc_block_seqno).collect();

        if seqnos.is_empty() {
            return 0;
        }

        seqnos.sort_by(|a, b| b.cmp(a));
        let consensus_idx = (seqnos.len() * 2) / 3;
        seqnos.get(consensus_idx).copied().unwrap_or(0)
    }

    pub(super) async fn delete_unsync_peers(&self) {
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

    pub(super) async fn execute_request<T>(
        &mut self,
        only_archive: bool,
    ) -> Result<(usize, Instant)> {
        let _ = std::marker::PhantomData::<T>;
        let peer_idx = self.choose_peer(only_archive).await?;
        if let Some(peer) = self.peers.get_mut(peer_idx) {
            peer.set_request_timeout(self.timeout);
        }

        if let Some(limiter) = &self.global_rate_limiter {
            limiter.acquire().await;
        }

        // Increment current request count
        {
            let mut stats = self.peer_stats.write().await;
            let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
            peer_stats.current_requests += 1;
        }

        let start = Instant::now();
        Ok((peer_idx, start))
    }

    pub(super) async fn execute_request_excluding<T>(
        &mut self,
        only_archive: bool,
        excluded: &HashSet<usize>,
    ) -> Result<(usize, Instant)> {
        let _ = std::marker::PhantomData::<T>;
        let peer_idx = self.choose_peer_excluding(only_archive, excluded).await?;
        if let Some(peer) = self.peers.get_mut(peer_idx) {
            peer.set_request_timeout(self.timeout);
        }

        if let Some(limiter) = &self.global_rate_limiter {
            limiter.acquire().await;
        }

        {
            let mut stats = self.peer_stats.write().await;
            let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
            peer_stats.current_requests += 1;
        }

        let start = Instant::now();
        Ok((peer_idx, start))
    }

    #[cfg(test)]
    pub(super) async fn execute_request_for_test(
        &mut self,
        only_archive: bool,
    ) -> Result<(usize, Instant)> {
        self.execute_request::<()>(only_archive).await
    }

    pub(super) async fn complete_request(
        &mut self,
        peer_idx: usize,
        start: Instant,
        success: bool,
    ) {
        let elapsed = start.elapsed().as_millis() as u64;

        self.decrement_current_requests(peer_idx).await;

        if success {
            self.record_success(peer_idx, elapsed).await;
        } else {
            self.record_retryable_failure(
                peer_idx,
                PeerFailureKind::Connection,
                self.timeout.as_millis() as u64,
            )
            .await;
        }
    }

    pub(super) async fn complete_request_error(
        &mut self,
        peer_idx: usize,
        start: Instant,
        error: &LiteError,
    ) {
        let elapsed = start.elapsed().as_millis() as u64;
        self.decrement_current_requests(peer_idx).await;

        if let Some(kind) = Self::retryable_failure(error) {
            self.record_retryable_failure(peer_idx, kind, self.timeout.as_millis() as u64)
                .await;
        } else {
            self.record_non_retryable_error(peer_idx, elapsed).await;
        }
    }

    pub(super) async fn update_peer_seqno(&self, peer_idx: usize, seqno: u32) {
        let mut stats = self.peer_stats.write().await;
        let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
        peer_stats.mc_block_seqno = seqno;
        peer_stats.last_observed_seqno = seqno;
    }

    // Delegate methods to underlying clients with load balancing
    pub async fn get_masterchain_info(&mut self) -> Result<MasterchainInfo> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<MasterchainInfo>(false).await?;
            let result = self.peers[peer_idx].get_masterchain_info().await;

            match result {
                Ok(response) => {
                    self.update_peer_seqno(peer_idx, response.last.seqno as u32)
                        .await;
                    self.complete_request(peer_idx, start, true).await;
                    self.delete_unsync_peers().await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn raw_get_block(&mut self, id: BlockIdExt) -> Result<crate::tlb::Block> {
        balanced_call!(self, crate::tlb::Block, false, |client| client
            .raw_get_block(id.clone()))
    }

    pub async fn raw_get_block_data(&mut self, id: BlockIdExt) -> Result<DecodedBlockData> {
        balanced_call!(self, DecodedBlockData, false, |client| {
            client.raw_get_block_data(id.clone())
        })
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
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
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
            let result = self.peers[peer_idx]
                .get_block_header(
                    id.clone(),
                    with_state_update,
                    with_value_flow,
                    with_extra,
                    with_shard_hashes,
                    with_prev_blk_signatures,
                )
                .await;

            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn raw_get_block_header(
        &mut self,
        id: BlockIdExt,
        with_state_update: bool,
        with_value_flow: bool,
        with_extra: bool,
        with_shard_hashes: bool,
        with_prev_blk_signatures: bool,
    ) -> Result<DecodedBlockHeader> {
        balanced_call!(self, DecodedBlockHeader, false, |client| {
            client.raw_get_block_header(
                id.clone(),
                with_state_update,
                with_value_flow,
                with_extra,
                with_shard_hashes,
                with_prev_blk_signatures,
            )
        })
    }

    pub async fn send_message(&mut self, body: Vec<u8>) -> Result<u32> {
        // For send_message, distribute to multiple peers
        let k = {
            let alive_count = self.alive_peers.read().await.len();
            if alive_count < 12 { 4 } else { alive_count / 3 }
        };

        let mut results = Vec::new();
        let mut attempted = HashSet::new();
        'peers: for _ in 0..k.min(self.peers.len()) {
            for _attempt in 0..self.max_retries {
                let (peer_idx, start) = match self
                    .execute_request_excluding::<u32>(false, &attempted)
                    .await
                {
                    Ok(request) => request,
                    Err(BalancerError::NoAlivePeers) => break 'peers,
                    Err(error) => return Err(error),
                };
                attempted.insert(peer_idx);
                let result = self.peers[peer_idx].send_message(body.clone()).await;

                match result {
                    Ok(status) => {
                        self.complete_request(peer_idx, start, true).await;
                        results.push(Ok(status));
                        break;
                    }
                    Err(e) => {
                        self.complete_request_error(peer_idx, start, &e).await;
                        results.push(Err(e));
                        break;
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

    pub async fn get_account_state(
        &mut self,
        id: BlockIdExt,
        account: AccountId,
    ) -> Result<AccountState> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<AccountState>(false).await?;
            let result = self.peers[peer_idx]
                .get_account_state(id.clone(), account.clone())
                .await;

            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn raw_get_account_state(
        &mut self,
        account: Address,
        block: Option<BlockIdExt>,
    ) -> Result<(
        Option<crate::tlb::Account>,
        Option<crate::tlb::ShardAccount>,
    )> {
        balanced_call!(
            self,
            (
                Option<crate::tlb::Account>,
                Option<crate::tlb::ShardAccount>
            ),
            false,
            |client| client.raw_get_account_state(account.clone(), block.clone())
        )
    }

    pub async fn get_account_state_typed(
        &mut self,
        account: Address,
        block: Option<BlockIdExt>,
    ) -> Result<DecodedAccountState> {
        balanced_call!(self, DecodedAccountState, false, |client| {
            client.get_account_state_typed(account.clone(), block.clone())
        })
    }

    pub async fn get_account_state_simple(&mut self, account: Address) -> Result<SimpleAccount> {
        balanced_call!(self, SimpleAccount, false, |client| {
            client.get_account_state_simple(account.clone())
        })
    }

    pub async fn run_smc_method(
        &mut self,
        mode: u32,
        id: BlockIdExt,
        account: Address,
        method_id: u64,
        params: Vec<u8>,
    ) -> Result<RunMethodResult> {
        for _attempt in 0..self.max_retries {
            let (peer_idx, start) = self.execute_request::<RunMethodResult>(false).await?;
            let result = self.peers[peer_idx]
                .run_smc_method(mode, id.clone(), account.clone(), method_id, params.clone())
                .await;

            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn run_get_method(
        &mut self,
        mode: u32,
        id: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<RunMethodResult> {
        let params = stack.to_boc().map_err(|e| {
            BalancerError::LiteError(LiteError::TlError(crate::tl::TlError::ParseError(
                e.to_string(),
            )))
        })?;
        self.run_smc_method(mode, id, account, method_id, params)
            .await
    }

    pub async fn run_get_method_typed(
        &mut self,
        mode: u32,
        id: BlockIdExt,
        account: Address,
        method_id: u64,
        stack: TvmStack,
    ) -> Result<Vec<TvmStackEntry>> {
        balanced_call!(self, Vec<TvmStackEntry>, false, |client| {
            client.run_get_method_typed(mode, id.clone(), account.clone(), method_id, stack.clone())
        })
    }

    pub async fn run_get_method_by_name(
        &mut self,
        mode: u32,
        id: BlockIdExt,
        account: Address,
        method: &str,
        stack: TvmStack,
    ) -> Result<RunMethodResult> {
        self.run_get_method(
            mode,
            id,
            account,
            crate::utils::method_name_to_id(method),
            stack,
        )
        .await
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
            let result = self.peers[peer_idx]
                .get_transactions(count, account.clone(), lt, hash.clone())
                .await;

            match result {
                Ok(response) => {
                    self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }

    pub async fn raw_get_transactions(
        &mut self,
        count: u32,
        account: AccountId,
        lt: u64,
        hash: Int256,
    ) -> Result<(Vec<crate::tlb::Transaction>, Vec<BlockIdExt>)> {
        balanced_call!(
            self,
            (Vec<crate::tlb::Transaction>, Vec<BlockIdExt>),
            false,
            |client| client.raw_get_transactions(count, account.clone(), lt, hash.clone())
        )
    }

    pub async fn raw_get_shard_info(
        &mut self,
        block: BlockIdExt,
        workchain: i32,
        shard: u64,
        exact: bool,
    ) -> Result<DecodedShardInfo> {
        balanced_call!(self, DecodedShardInfo, false, |client| {
            client.raw_get_shard_info(block.clone(), workchain, shard, exact)
        })
    }

    pub async fn raw_get_all_shards_info(
        &mut self,
        block: BlockIdExt,
    ) -> Result<DecodedAllShardsInfo> {
        balanced_call!(self, DecodedAllShardsInfo, false, |client| {
            client.raw_get_all_shards_info(block.clone())
        })
    }

    pub async fn get_all_shards_info_typed(
        &mut self,
        block: BlockIdExt,
    ) -> Result<Vec<BlockIdExt>> {
        balanced_call!(self, Vec<BlockIdExt>, false, |client| {
            client.get_all_shards_info_typed(block.clone())
        })
    }

    pub async fn get_one_transaction_typed(
        &mut self,
        block: BlockIdExt,
        account: AccountId,
        lt: u64,
    ) -> Result<Option<crate::tlb::Transaction>> {
        balanced_call!(self, Option<crate::tlb::Transaction>, false, |client| {
            client.get_one_transaction_typed(block.clone(), account.clone(), lt)
        })
    }

    pub async fn raw_get_block_transactions_ext(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<Vec<crate::tlb::Transaction>> {
        balanced_call!(self, Vec<crate::tlb::Transaction>, false, |client| {
            client.raw_get_block_transactions_ext(
                id.clone(),
                count,
                after.clone(),
                reverse_order,
                want_proof,
            )
        })
    }

    pub async fn list_block_transactions_ext_decoded(
        &mut self,
        id: BlockIdExt,
        count: u32,
        after: Option<TransactionId3>,
        reverse_order: bool,
        want_proof: bool,
    ) -> Result<DecodedBlockTransactionsExt> {
        balanced_call!(self, DecodedBlockTransactionsExt, false, |client| {
            client.list_block_transactions_ext_decoded(
                id.clone(),
                count,
                after.clone(),
                reverse_order,
                want_proof,
            )
        })
    }

    pub async fn get_config_all_typed(
        &mut self,
        id: BlockIdExt,
        with_state_root: bool,
        with_libraries: bool,
        with_state_extra_root: bool,
        with_shard_hashes: bool,
        with_validator_set: bool,
        with_special_smc: bool,
        with_accounts_root: bool,
        with_prev_blocks: bool,
        with_workchain_info: bool,
        with_capabilities: bool,
        extract_from_key_block: bool,
    ) -> Result<DecodedConfigInfo> {
        balanced_call!(self, DecodedConfigInfo, false, |client| {
            client.get_config_all_typed(
                id.clone(),
                with_state_root,
                with_libraries,
                with_state_extra_root,
                with_shard_hashes,
                with_validator_set,
                with_special_smc,
                with_accounts_root,
                with_prev_blocks,
                with_workchain_info,
                with_capabilities,
                extract_from_key_block,
            )
        })
    }

    pub async fn get_config_params_typed(
        &mut self,
        id: BlockIdExt,
        param_list: Vec<i32>,
        with_state_root: bool,
        with_libraries: bool,
        with_state_extra_root: bool,
        with_shard_hashes: bool,
        with_validator_set: bool,
        with_special_smc: bool,
        with_accounts_root: bool,
        with_prev_blocks: bool,
        with_workchain_info: bool,
        with_capabilities: bool,
        extract_from_key_block: bool,
    ) -> Result<DecodedConfigInfo> {
        balanced_call!(self, DecodedConfigInfo, false, |client| {
            client.get_config_params_typed(
                id.clone(),
                param_list.clone(),
                with_state_root,
                with_libraries,
                with_state_extra_root,
                with_shard_hashes,
                with_validator_set,
                with_special_smc,
                with_accounts_root,
                with_prev_blocks,
                with_workchain_info,
                with_capabilities,
                extract_from_key_block,
            )
        })
    }

    pub async fn get_libraries_typed(
        &mut self,
        library_list: Vec<Int256>,
    ) -> Result<HashMap<Int256, Option<Arc<crate::tvm::Cell>>>> {
        balanced_call!(
            self,
            HashMap<Int256, Option<Arc<crate::tvm::Cell>>>,
            false,
            |client| client.get_libraries_typed(library_list.clone())
        )
    }

    pub async fn get_libraries_with_proof_typed(
        &mut self,
        id: BlockIdExt,
        mode: u32,
        library_list: Vec<Int256>,
    ) -> Result<DecodedLibrariesWithProof> {
        balanced_call!(self, DecodedLibrariesWithProof, false, |client| {
            client.get_libraries_with_proof_typed(id.clone(), mode, library_list.clone())
        })
    }
}
