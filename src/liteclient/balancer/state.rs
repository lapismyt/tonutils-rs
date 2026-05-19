use super::*;

impl LiteBalancer {
    pub(super) fn retryable_failure(error: &LiteError) -> Option<PeerFailureKind> {
        match error {
            LiteError::Timeout { .. } => Some(PeerFailureKind::Timeout),
            LiteError::AdnlError(crate::adnl::helper_types::AdnlError::Timeout { .. }) => {
                Some(PeerFailureKind::Timeout)
            }
            LiteError::AdnlError(_) => Some(PeerFailureKind::Connection),
            LiteError::ServerError(_)
            | LiteError::TlError(_)
            | LiteError::UnexpectedMessage
            | LiteError::UnknownError(_) => None,
        }
    }

    pub(super) async fn decrement_current_requests(&self, peer_idx: usize) {
        let mut stats = self.peer_stats.write().await;
        if let Some(peer_stats) = stats.get_mut(&peer_idx) {
            peer_stats.current_requests = peer_stats.current_requests.saturating_sub(1);
        }
    }

    pub(super) async fn record_success(&self, peer_idx: usize, elapsed_ms: u64) {
        self.update_average_request_time(peer_idx, elapsed_ms).await;

        {
            let mut stats = self.peer_stats.write().await;
            let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
            peer_stats.failure_count = 0;
            peer_stats.last_failure_kind = None;
        }

        self.alive_peers.write().await.insert(peer_idx);
        self.peer_states
            .write()
            .await
            .insert(peer_idx, PeerState::Healthy);
    }

    pub(super) async fn record_retryable_failure(
        &self,
        peer_idx: usize,
        kind: PeerFailureKind,
        request_time_ms: u64,
    ) {
        let failure_count = {
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
            peer_stats.failure_count += 1;
            peer_stats.last_failure_kind = Some(kind);
            peer_stats.failure_count
        };

        if failure_count >= 2 {
            self.alive_peers.write().await.remove(&peer_idx);
            self.peer_states
                .write()
                .await
                .insert(peer_idx, PeerState::Dead);
        } else {
            self.alive_peers.write().await.insert(peer_idx);
            self.peer_states
                .write()
                .await
                .insert(peer_idx, PeerState::Suspect);
        }
    }

    pub(super) async fn record_non_retryable_error(&self, peer_idx: usize, elapsed_ms: u64) {
        let mut stats = self.peer_stats.write().await;
        let peer_stats = stats.entry(peer_idx).or_insert_with(PeerStats::default);
        peer_stats.avg_response_time_ms = Self::calc_new_average(
            peer_stats.avg_response_time_ms,
            peer_stats.total_requests,
            elapsed_ms,
        );
        peer_stats.ewma_latency_ms = Some(match peer_stats.ewma_latency_ms {
            Some(old) => (old * 7 + elapsed_ms * 3) / 10,
            None => elapsed_ms,
        });
        peer_stats.total_requests += 1;
    }
}
