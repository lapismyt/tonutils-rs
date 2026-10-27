use std::task::Poll;

use crate::{
    adnl::AdnlError,
    liteclient::types::LiteError,
    tl::{TlError, adnl::AdnlMessage, common::Int256},
};

use futures::{Sink, Stream};
use pin_project::pin_project;
use rand::random;
use tokio_tower::multiplex::TagStore;
use tokio_util::bytes::Bytes;

#[pin_project]
pub struct LitePeer<T> {
    #[pin]
    inner: T,
}

impl<T> LitePeer<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T> Sink<AdnlMessage> for LitePeer<T>
where
    T: Sink<Bytes, Error = AdnlError>,
{
    type Error = LiteError;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.project()
            .inner
            .poll_ready(cx)
            .map_err(LiteError::AdnlError)
    }

    fn start_send(self: std::pin::Pin<&mut Self>, item: AdnlMessage) -> Result<(), Self::Error> {
        log::debug!("Sending TL message: {:?}", item);
        let data = tl_proto::serialize(item).into();
        self.project()
            .inner
            .start_send(data)
            .map_err(LiteError::AdnlError)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.project()
            .inner
            .poll_flush(cx)
            .map_err(LiteError::AdnlError)
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.project()
            .inner
            .poll_close(cx)
            .map_err(LiteError::AdnlError)
    }
}

impl<T> Stream for LitePeer<T>
where
    T: Stream<Item = Result<Bytes, AdnlError>>,
{
    type Item = Result<AdnlMessage, LiteError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let inner = self.project().inner.poll_next(cx);
        match inner {
            Poll::Ready(Some(Ok(bytes))) => {
                let decoded = tl_proto::deserialize(&bytes);
                log::debug!("Decoded to TL message:\n{:?}\n{:?}", bytes, decoded);
                Poll::Ready(Some(decoded.map_err(|e| {
                    LiteError::TlError(TlError::ParseError(e.to_string()))
                })))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(LiteError::AdnlError(e)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum LiteTag {
    Int256(Int256),
    Long(u64),
}

impl<T> TagStore<AdnlMessage, AdnlMessage> for LitePeer<T> {
    type Tag = LiteTag;

    fn assign_tag(self: std::pin::Pin<&mut Self>, r: &mut AdnlMessage) -> Self::Tag {
        match r {
            AdnlMessage::Answer { query_id, .. } => {
                *query_id = Int256::random();
                LiteTag::Int256(query_id.clone())
            }
            AdnlMessage::Query { query_id, .. } => {
                *query_id = Int256::random();
                LiteTag::Int256(query_id.clone())
            }
            AdnlMessage::Ping { random_id } => {
                *random_id = random();
                LiteTag::Long(*random_id)
            }
            AdnlMessage::Pong { random_id } => {
                *random_id = random();
                LiteTag::Long(*random_id)
            }
            _ => unreachable!("LitePeer cannot tag non-LiteAPI ADNL messages"),
        }
    }

    fn finish_tag(self: std::pin::Pin<&mut Self>, r: &AdnlMessage) -> Self::Tag {
        match r {
            AdnlMessage::Answer { query_id, .. } => LiteTag::Int256(query_id.clone()),
            AdnlMessage::Query { query_id, .. } => LiteTag::Int256(query_id.clone()),
            AdnlMessage::Ping { random_id } => LiteTag::Long(*random_id),
            AdnlMessage::Pong { random_id } => LiteTag::Long(*random_id),
            _ => unreachable!("LitePeer cannot finish non-LiteAPI ADNL messages"),
        }
    }
}
