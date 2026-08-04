use std::time::Duration;
use thiserror::Error;
use tonutils_adnl::helper_types::AdnlError;
use tonutils_tl::TlError;
use tower::Service;

use tonutils_tl::{request::WrappedRequest, response::Response};

#[derive(Debug, Error)]
pub enum LiteError {
    #[error("Liteserver error {0}")]
    ServerError(tonutils_tl::response::Error),
    #[error("{operation} timed out after {timeout:?}")]
    Timeout {
        operation: &'static str,
        timeout: Duration,
    },
    #[error("TL parsing error: {0}")]
    TlError(TlError),
    #[error("Unexpected TL message")]
    UnexpectedMessage,
    #[error("ADNL error: {0}")]
    AdnlError(#[from] AdnlError),
    #[error("Unknown error: {0}")]
    UnknownError(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub trait LiteService: Service<WrappedRequest, Response = Response, Error = LiteError>
where
    Self::Future: Send + 'static,
{
}

impl<T> LiteService for T
where
    T: Service<WrappedRequest, Response = Response, Error = LiteError>,
    T::Future: Send + 'static,
{
}
