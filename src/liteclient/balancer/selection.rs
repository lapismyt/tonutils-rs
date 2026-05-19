use super::*;

impl LiteBalancer {
    pub(super) fn peer_score(stats: Option<&PeerStats>, best_seqno: u32, timeout_ms: u64) -> u64 {
        let Some(stats) = stats else {
            return timeout_ms;
        };
        let latency = stats.ewma_latency_ms.unwrap_or({
            if stats.avg_response_time_ms == 0 {
                timeout_ms
            } else {
                stats.avg_response_time_ms
            }
        });
        let stale_penalty = u64::from(
            best_seqno.saturating_sub(stats.last_observed_seqno.max(stats.mc_block_seqno)),
        ) * timeout_ms.max(1)
            * 2;
        let in_flight_penalty = stats
            .current_requests
            .saturating_mul(timeout_ms.max(1) / 4 + 1);

        latency + stale_penalty + in_flight_penalty
    }
}
