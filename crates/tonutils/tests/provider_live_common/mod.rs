use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use tonutils::provider::ProviderError;

/// Toncenter allows a single request per second without an API key.
pub const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(1_200);

pub const PROVIDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Blocks every provider live test until the keyless rate limit permits another
/// request. Tests may run in parallel, but HTTP calls stay one per interval.
pub async fn request_slot() {
    static LAST_REQUEST: OnceLock<tokio::sync::Mutex<Option<Instant>>> = OnceLock::new();
    let mut last_request = LAST_REQUEST
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if let Some(previous) = *last_request
        && let Some(remaining) = MIN_REQUEST_INTERVAL.checked_sub(previous.elapsed())
    {
        tokio::time::sleep(remaining).await;
    }
    *last_request = Some(Instant::now());
}

pub fn is_retryable(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Api { status, .. } if matches!(status, 429 | 502 | 503 | 504)
    )
}

pub fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(1_u64 << attempt.min(8))
}

pub fn elapsed_millis(started: Instant) -> u128 {
    started.elapsed().as_millis()
}
