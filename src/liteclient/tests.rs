//! Tests for liteclient module

use super::types::*;
use crate::tl::{BlockIdExt, Int256, TlError};
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
fn test_lite_error_display_includes_tl_details() {
    let err = LiteError::TlError(TlError::ParseError("bad constructor".to_owned()));

    assert_eq!(err.to_string(), "TL parsing error: bad constructor");
}

#[test]
fn test_lite_error_display_includes_server_details() {
    let err = LiteError::ServerError(crate::tl::response::Error {
        code: 400,
        message: "bad request".into(),
    });

    assert_eq!(
        err.to_string(),
        "Liteserver error code=400, message=bad request"
    );
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

fn test_block_id() -> BlockIdExt {
    BlockIdExt {
        workchain: -1,
        shard: i64::MIN,
        seqno: 1,
        root_hash: Int256([1; 32]),
        file_hash: Int256([2; 32]),
    }
}

fn response_client(response: crate::tl::response::Response) -> super::client::LiteClient {
    let bytes = tl_proto::serialize(response);
    super::client::LiteClient::from_service(service_fn(
        move |_request: crate::tl::request::RawWrappedRequest| {
            let bytes = bytes.clone();
            async move { Ok::<_, LiteError>(bytes) }
        },
    ))
}

#[tokio::test]
async fn raw_get_block_decodes_block_boc() {
    use crate::tlb::{Block, TlbSerialize};
    use crate::tvm::{Builder, serialize_boc};

    let child = Builder::new().build().unwrap();
    let block = Block {
        global_id: -239,
        info: child.clone(),
        value_flow: child.clone(),
        state_update: child.clone(),
        extra: child,
    };
    let data = serialize_boc(&block.to_cell().unwrap(), false).unwrap();
    let id = test_block_id();
    let mut client = response_client(crate::tl::response::Response::BlockData(
        crate::tl::response::BlockData { id, data },
    ));

    assert_eq!(client.raw_get_block(test_block_id()).await.unwrap(), block);
}

#[tokio::test]
async fn get_account_state_typed_extracts_simple_account() {
    use crate::tlb::{Account, TlbSerialize};
    use crate::tvm::{Address, serialize_boc};

    let account = Account::None;
    let state = serialize_boc(&account.to_cell().unwrap(), false).unwrap();
    let id = test_block_id();
    let mut client = response_client(crate::tl::response::Response::AccountState(
        crate::tl::response::AccountState {
            id: id.clone(),
            shardblk: id,
            shard_proof: Vec::new(),
            proof: Vec::new(),
            state,
        },
    ));

    let decoded = client
        .get_account_state_typed(Address::new(0, [0; 32]), Some(test_block_id()))
        .await
        .unwrap();
    let simple = decoded.simple();
    assert_eq!(simple.state, super::boc::SimpleAccountState::None);
    assert_eq!(simple.last_transaction_lt, None);
    assert_eq!(simple.last_transaction_hash, None);
}

#[tokio::test]
async fn run_get_method_typed_rejects_nonzero_exit_code() {
    use crate::tvm::{Address, TvmStack};

    let id = test_block_id();
    let mut client = response_client(crate::tl::response::Response::RunMethodResult(
        crate::tl::response::RunMethodResult {
            mode: (),
            id: id.clone(),
            shardblk: id,
            shard_proof: None,
            proof: None,
            state_proof: None,
            init_c7: None,
            lib_extras: None,
            exit_code: 42,
            result: None,
        },
    ));

    let error = client
        .run_get_method_typed(
            0,
            test_block_id(),
            Address::new(0, [0; 32]),
            1,
            TvmStack::empty(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("TL parsing error"));
}
