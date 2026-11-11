use crate::{
    liteclient::{balancer::BalancerError, types::LiteError},
    tvm::TvmError,
};

/// Error returned by provider capabilities.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid provider URL: {0}")]
    Url(String),
    #[error("{provider} returned HTTP {status}: {message}")]
    Api {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("failed to decode {provider} response: {source}")]
    Decode {
        provider: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid {provider} response: {message}")]
    InvalidResponse {
        provider: &'static str,
        message: String,
    },
    #[error("{provider} does not support reading an explicit block")]
    UnsupportedBlock { provider: &'static str },
    #[error("{provider} does not support numeric method ids; use a method name")]
    UnsupportedMethodRef { provider: &'static str },
    #[error("provider does not support this TVM stack entry: {kind}")]
    UnsupportedStackEntry { kind: &'static str },
    #[error("invalid TVM stack value: {reason}")]
    InvalidStack { reason: String },
    #[error("TVM error: {0}")]
    Tvm(#[from] TvmError),
    #[error("LiteClient error: {0}")]
    Lite(#[from] LiteError),
    #[error("LiteBalancer error: {0}")]
    Balancer(#[from] BalancerError),
}

pub type ProviderResult<T> = Result<T, ProviderError>;
