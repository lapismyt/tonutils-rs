use std::task::{Context, Poll};

use futures::future::{self, BoxFuture};
use tower::{Layer, Service};

use crate::liteclient::types::{LiteError, LiteService};
use crate::tl::adnl::Message;
use crate::tl::common::Int256;
use crate::tl::request::{LiteQuery, RawWrappedRequest, WrappedRequest};
use crate::tl::response::{Error, Response};

pub struct WrapMessagesLayer;

impl<S> Layer<S> for WrapMessagesLayer {
    type Service = WrapService<S>;

    fn layer(&self, service: S) -> Self::Service {
        WrapService { service }
    }
}

pub struct WrapService<S> {
    service: S,
}

impl<S> Service<WrappedRequest> for WrapService<S>
where
    S: Service<Message>,
    S::Error: Into<LiteError>,
    S::Response: Into<Message>,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = LiteError;
    type Future = BoxFuture<'static, Result<Response, LiteError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: WrappedRequest) -> Self::Future {
        let fut = self.service.call(Message::Query {
            query_id: Int256::default(),
            query: tl_proto::serialize(LiteQuery {
                wrapped_request: request,
            }),
        });
        Box::pin(async move {
            let response = fut.await.map_err(Into::into)?.into();

            match response {
                Message::Answer { answer, .. } => {
                    let response = tl_proto::deserialize(&answer).map_err(|e| {
                        LiteError::TlError(crate::tl::TlError::ParseError(e.to_string()))
                    })?;
                    Ok(response)
                }
                _ => Err(LiteError::UnexpectedMessage),
            }
        })
    }
}

pub struct WrapRawMessagesLayer;

impl<S> Layer<S> for WrapRawMessagesLayer {
    type Service = WrapRawService<S>;

    fn layer(&self, service: S) -> Self::Service {
        WrapRawService { service }
    }
}

pub struct WrapRawService<S> {
    service: S,
}

impl<S> Service<RawWrappedRequest> for WrapRawService<S>
where
    S: Service<Message>,
    S::Error: Into<LiteError>,
    S::Response: Into<Message>,
    S::Future: Send + 'static,
{
    type Response = Vec<u8>;
    type Error = LiteError;
    type Future = BoxFuture<'static, Result<Vec<u8>, LiteError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: RawWrappedRequest) -> Self::Future {
        let fut = self.service.call(Message::Query {
            query_id: Int256::default(),
            query: tl_proto::serialize(request.into_lite_query()),
        });
        Box::pin(async move {
            let response = fut.await.map_err(Into::into)?.into();

            match response {
                Message::Answer { answer, .. } => Ok(answer),
                _ => Err(LiteError::UnexpectedMessage),
            }
        })
    }
}

pub struct UnwrapMessagesLayer;

impl<S> Layer<S> for UnwrapMessagesLayer {
    type Service = UnwrapService<S>;

    fn layer(&self, service: S) -> Self::Service {
        UnwrapService { service }
    }
}

pub struct UnwrapService<S> {
    service: S,
}

impl<S> Service<Message> for UnwrapService<S>
where
    S: LiteService,
    S::Future: Send + 'static,
{
    type Response = Message;
    type Error = LiteError;
    type Future = BoxFuture<'static, Result<Message, LiteError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: Message) -> Self::Future {
        let (query_id, request) = match request {
            Message::Query { query_id, query } => {
                let decoded: Result<LiteQuery, LiteError> = tl_proto::deserialize(&query)
                    .map_err(|e| LiteError::TlError(crate::tl::TlError::ParseError(e.to_string())));
                match decoded {
                    Ok(LiteQuery { wrapped_request }) => (query_id, wrapped_request),
                    Err(e) => return Box::pin(future::err(e)),
                }
            }
            Message::Ping { random_id } => {
                return Box::pin(future::ok(Message::Pong { random_id }));
            }
            _ => return Box::pin(future::err(LiteError::UnexpectedMessage)),
        };
        let fut = self.service.call(request);
        Box::pin(async move {
            let answer: Response = fut.await.map_err(Into::<LiteError>::into)?.into();
            Ok(Message::Answer {
                query_id,
                answer: tl_proto::serialize(answer),
            })
        })
    }
}

pub struct WrapErrorLayer;

impl<S> Layer<S> for WrapErrorLayer {
    type Service = WrapErrorService<S>;

    fn layer(&self, service: S) -> Self::Service {
        WrapErrorService { service }
    }
}

pub struct WrapErrorService<S> {
    service: S,
}

impl<S> Service<WrappedRequest> for WrapErrorService<S>
where
    S: Service<WrappedRequest>,
    S::Error: Into<LiteError>,
    S::Response: Into<Response>,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = LiteError;
    type Future = BoxFuture<'static, Result<Response, LiteError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, request: WrappedRequest) -> Self::Future {
        let fut = self.service.call(request);
        Box::pin(async move {
            let response = fut.await;
            match response {
                Ok(x) => Ok(x.into()),
                Err(e) => Ok(Response::Error(Error {
                    code: 500,
                    message: format!("{:?}", e.into()).as_str().into(),
                })),
            }
        })
    }
}

pub struct UnwrapErrorService<S> {
    service: S,
}

impl<S> UnwrapErrorService<S> {
    pub fn new(service: S) -> Self {
        Self { service }
    }
}

impl<S> Service<WrappedRequest> for UnwrapErrorService<S>
where
    S: Service<WrappedRequest, Response = Response, Error = LiteError>,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = LiteError;
    type Future = BoxFuture<'static, Result<Response, LiteError>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: WrappedRequest) -> Self::Future {
        let fut = self.service.call(request);
        Box::pin(async move {
            match fut.await {
                Ok(Response::Error(error)) => Err(LiteError::from(error)),
                Ok(response) => Ok(response),
                Err(e) => Err(e),
            }
        })
    }
}

// Implement From<Error> for LiteError
impl From<Error> for LiteError {
    fn from(error: Error) -> Self {
        LiteError::ServerError(error)
    }
}

pub struct UnwrapErrorLayer;

impl<S> Layer<S> for UnwrapErrorLayer {
    type Service = UnwrapErrorService<S>;

    fn layer(&self, service: S) -> Self::Service {
        UnwrapErrorService { service }
    }
}

#[cfg(test)]
mod tests {
    use tower::{Layer, Service, service_fn};

    use super::*;
    use crate::tl::request::LiteQueryRaw;

    #[tokio::test]
    async fn test_wrap_raw_messages_layer_preserves_unknown_request_and_response_bytes() {
        let service = service_fn(|message: Message| async move {
            match message {
                Message::Query { query, query_id } => {
                    let query: LiteQueryRaw = tl_proto::deserialize(&query).unwrap();
                    assert_eq!(query.data, vec![1, 2, 3, 4]);
                    Ok::<_, LiteError>(Message::Answer {
                        query_id,
                        answer: vec![9, 8, 7],
                    })
                }
                _ => Err(LiteError::UnexpectedMessage),
            }
        });
        let mut service = WrapRawMessagesLayer.layer(service);

        let response = service
            .call(RawWrappedRequest {
                wait_masterchain_seqno: None,
                request: vec![1, 2, 3, 4],
            })
            .await
            .unwrap();

        assert_eq!(response, vec![9, 8, 7]);
    }
}
