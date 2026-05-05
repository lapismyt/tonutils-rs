use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::Mutex;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRateLimit {
    pub rps: u32,
    pub burst: u32,
}

impl RequestRateLimit {
    pub fn per_second(rps: u32) -> Result<Self, RateLimitError> {
        Self::with_burst(rps, rps)
    }

    pub fn with_burst(rps: u32, burst: u32) -> Result<Self, RateLimitError> {
        if rps == 0 {
            return Err(RateLimitError::ZeroRps);
        }
        if burst == 0 {
            return Err(RateLimitError::ZeroBurst);
        }
        Ok(Self { rps, burst })
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitError {
    #[error("request rate limit rps must be greater than zero")]
    ZeroRps,
    #[error("request rate limit burst must be greater than zero")]
    ZeroBurst,
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    start: std::time::Instant,
    bucket: Arc<Mutex<TokenBucket>>,
}

impl RateLimiter {
    pub fn new(limit: RequestRateLimit) -> Self {
        Self {
            start: std::time::Instant::now(),
            bucket: Arc::new(Mutex::new(TokenBucket::new(limit, Duration::ZERO))),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;
                bucket.acquire_at(self.start.elapsed())
            };

            match wait {
                None => return,
                Some(duration) => tokio::time::sleep(duration).await,
            }
        }
    }
}

#[derive(Debug)]
struct TokenBucket {
    limit: RequestRateLimit,
    tokens: u32,
    last_refill_ns: u128,
}

impl TokenBucket {
    fn new(limit: RequestRateLimit, now: Duration) -> Self {
        Self {
            limit,
            tokens: limit.burst,
            last_refill_ns: now.as_nanos(),
        }
    }

    fn acquire_at(&mut self, now: Duration) -> Option<Duration> {
        let now_ns = now.as_nanos();
        self.refill(now_ns);

        if self.tokens > 0 {
            self.tokens -= 1;
            None
        } else {
            Some(self.wait_duration(now_ns))
        }
    }

    fn refill(&mut self, now_ns: u128) {
        let elapsed_ns = now_ns.saturating_sub(self.last_refill_ns);
        let new_tokens = elapsed_ns.saturating_mul(self.limit.rps as u128) / NANOS_PER_SECOND;
        if new_tokens == 0 {
            return;
        }

        self.tokens = self
            .tokens
            .saturating_add(new_tokens.min(u32::MAX as u128) as u32)
            .min(self.limit.burst);
        self.last_refill_ns += new_tokens.saturating_mul(NANOS_PER_SECOND) / self.limit.rps as u128;
    }

    fn wait_duration(&self, now_ns: u128) -> Duration {
        let nanos_per_token = NANOS_PER_SECOND.div_ceil(self.limit.rps as u128);
        let elapsed_ns = now_ns.saturating_sub(self.last_refill_ns);
        let wait_ns = nanos_per_token.saturating_sub(elapsed_ns).max(1);
        Duration::from_nanos(wait_ns.min(u64::MAX as u128) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_rps() {
        assert_eq!(
            RequestRateLimit::per_second(0).unwrap_err(),
            RateLimitError::ZeroRps
        );
    }

    #[test]
    fn rejects_zero_burst() {
        assert_eq!(
            RequestRateLimit::with_burst(1, 0).unwrap_err(),
            RateLimitError::ZeroBurst
        );
    }

    #[test]
    fn per_second_sets_burst_to_rps() {
        assert_eq!(
            RequestRateLimit::per_second(7).unwrap(),
            RequestRateLimit { rps: 7, burst: 7 }
        );
    }

    #[test]
    fn burst_permits_exact_immediate_acquisitions() {
        let limit = RequestRateLimit::with_burst(2, 3).unwrap();
        let mut bucket = TokenBucket::new(limit, Duration::ZERO);

        assert_eq!(bucket.acquire_at(Duration::ZERO), None);
        assert_eq!(bucket.acquire_at(Duration::ZERO), None);
        assert_eq!(bucket.acquire_at(Duration::ZERO), None);
        assert_eq!(
            bucket.acquire_at(Duration::ZERO),
            Some(Duration::from_millis(500))
        );
    }

    #[test]
    fn refill_restores_tokens_from_elapsed_time() {
        let limit = RequestRateLimit::with_burst(4, 4).unwrap();
        let mut bucket = TokenBucket::new(limit, Duration::ZERO);

        for _ in 0..4 {
            assert_eq!(bucket.acquire_at(Duration::ZERO), None);
        }
        assert!(bucket.acquire_at(Duration::ZERO).is_some());
        assert_eq!(bucket.acquire_at(Duration::from_millis(250)), None);
    }

    #[test]
    fn steady_state_spacing_matches_rps() {
        let limit = RequestRateLimit::with_burst(5, 1).unwrap();
        let mut bucket = TokenBucket::new(limit, Duration::ZERO);

        assert_eq!(bucket.acquire_at(Duration::ZERO), None);
        assert_eq!(
            bucket.acquire_at(Duration::ZERO),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            bucket.acquire_at(Duration::from_millis(199)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(bucket.acquire_at(Duration::from_millis(200)), None);
    }
}
