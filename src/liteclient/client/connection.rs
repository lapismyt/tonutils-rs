use super::*;

use crate::liteclient::{rate_limit::RateLimiter, types::LiteError};

pub struct LiteClient {
    pub(super) inner: tower::util::BoxService<RawWrappedRequest, Vec<u8>, LiteError>,
    pub(super) wait_seqno: Option<u32>,
    pub(super) rate_limiter: Option<RateLimiter>,
    pub(super) request_timeout: Option<std::time::Duration>,
}
