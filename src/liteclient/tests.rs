//! Tests for liteclient module

use super::types::*;
use crate::tl::TlError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::service_fn;

#[test]
fn test_lite_error_tl_error() {
    let tl_err = TlError::UnexpectedEof;
    let lite_err = LiteError::TlError(tl_err);

    match lite_err {
        LiteError::TlError(_) => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}

#[test]
fn test_lite_error_unexpected_message() {
    let err = LiteError::UnexpectedMessage;

    match err {
        LiteError::UnexpectedMessage => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}

#[test]
fn test_lite_error_display() {
    let err = LiteError::UnexpectedMessage;
    let error_string = format!("{}", err);

    assert!(error_string.contains("Unexpected"));
}

#[test]
fn test_lite_error_debug() {
    let err = LiteError::UnexpectedMessage;
    let debug_string = format!("{:?}", err);

    assert!(debug_string.contains("UnexpectedMessage"));
}

#[test]
fn test_tl_error_unexpected_eof() {
    let err = TlError::UnexpectedEof;
    let debug_str = format!("{:?}", err);

    assert!(debug_str.contains("UnexpectedEof"));
}

#[test]
fn test_lite_error_from_adnl_error() {
    use crate::adnl::helper_types::AdnlError;

    let adnl_err = AdnlError::IntegrityError;
    let lite_err: LiteError = adnl_err.into();

    match lite_err {
        LiteError::AdnlError(_) => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}

#[tokio::test]
async fn query_raw_waits_through_limiter_before_calling_service() {
    let calls = Arc::new(Mutex::new(0usize));
    let service_calls = Arc::clone(&calls);
    let service = service_fn(move |request: crate::tl::request::RawWrappedRequest| {
        let service_calls = Arc::clone(&service_calls);
        async move {
            *service_calls.lock().await += 1;
            Ok::<_, LiteError>(request.request)
        }
    });
    let mut client = super::client::LiteClient::from_service(service)
        .with_rate_limit(super::rate_limit::RequestRateLimit::with_burst(1, 1).unwrap());

    assert_eq!(client.query_raw([1]).await.unwrap(), vec![1]);
    assert_eq!(*calls.lock().await, 1);

    let pending =
        tokio::time::timeout(std::time::Duration::from_millis(10), client.query_raw([2])).await;
    assert!(pending.is_err());
    assert_eq!(*calls.lock().await, 1);
}

#[tokio::test]
async fn cancellation_before_limiter_acquisition_does_not_consume_wait_seqno() {
    let wait_seqnos = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&wait_seqnos);
    let service = service_fn(move |request: crate::tl::request::RawWrappedRequest| {
        let captured = Arc::clone(&captured);
        async move {
            captured
                .lock()
                .await
                .push(request.wait_masterchain_seqno.map(|wait| wait.seqno));
            Ok::<_, LiteError>(request.request)
        }
    });
    let mut client = super::client::LiteClient::from_service(service)
        .with_rate_limit(super::rate_limit::RequestRateLimit::with_burst(1, 1).unwrap());

    assert_eq!(client.query_raw([1]).await.unwrap(), vec![1]);
    let mut client = client.wait_masterchain_seqno(42);

    let pending =
        tokio::time::timeout(std::time::Duration::from_millis(10), client.query_raw([2])).await;
    assert!(pending.is_err());

    client.clear_rate_limit();
    assert_eq!(client.query_raw([3]).await.unwrap(), vec![3]);
    assert_eq!(*wait_seqnos.lock().await, vec![None, Some(42)]);
}
