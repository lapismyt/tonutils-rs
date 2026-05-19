//! Tests for liteclient module

use super::types::*;
use crate::tl::{
    BlockId, BlockIdExt, Int256, TlError, ZeroStateIdExt, common::LibraryEntry, request::Request,
    response::Response,
};
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

fn test_zero_state_id() -> ZeroStateIdExt {
    ZeroStateIdExt {
        workchain: -1,
        root_hash: Int256([3; 32]),
        file_hash: Int256([4; 32]),
    }
}

fn response_client(response: Response) -> super::client::LiteClient {
    let bytes = tl_proto::serialize(response);
    super::client::LiteClient::from_service(service_fn(
        move |_request: crate::tl::request::RawWrappedRequest| {
            let bytes = bytes.clone();
            async move { Ok::<_, LiteError>(bytes) }
        },
    ))
}

fn request_response_client(
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    response: Response,
) -> super::client::LiteClient {
    let bytes = tl_proto::serialize(response);
    super::client::LiteClient::from_service(service_fn(
        move |request: crate::tl::request::RawWrappedRequest| {
            let requests = Arc::clone(&requests);
            let bytes = bytes.clone();
            async move {
                requests.lock().await.push(request.request);
                Ok::<_, LiteError>(bytes)
            }
        },
    ))
}

#[tokio::test]
async fn query_raw_preserves_unknown_request_and_response_bytes() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let service_captured = Arc::clone(&captured);
    let response = vec![0xde, 0xad, 0xbe, 0xef, 0x00];
    let service_response = response.clone();
    let service = service_fn(move |request: crate::tl::request::RawWrappedRequest| {
        let service_captured = Arc::clone(&service_captured);
        let service_response = service_response.clone();
        async move {
            service_captured.lock().await.push(request.request);
            Ok::<_, LiteError>(service_response)
        }
    });
    let mut client = super::client::LiteClient::from_service(service);

    let request = vec![0xfe, 0xed, 0xfa, 0xce, 0x01, 0x02];
    assert_eq!(client.query_raw(&request).await.unwrap(), response);
    assert_eq!(*captured.lock().await, vec![request]);
}

#[tokio::test]
async fn query_raw_request_timeout_maps_to_lite_error_timeout() {
    let service = service_fn(
        move |_request: crate::tl::request::RawWrappedRequest| async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<_, LiteError>(Vec::new())
        },
    );
    let mut client = super::client::LiteClient::from_service(service)
        .with_request_timeout(std::time::Duration::from_millis(10));

    let error = client.query_raw([1]).await.unwrap_err();

    assert!(matches!(
        error,
        LiteError::Timeout {
            operation: "request_call",
            timeout
        } if timeout == std::time::Duration::from_millis(10)
    ));
}

#[tokio::test]
async fn query_raw_without_request_timeout_preserves_existing_behavior() {
    let service = service_fn(
        move |request: crate::tl::request::RawWrappedRequest| async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            Ok::<_, LiteError>(request.request)
        },
    );
    let mut client = super::client::LiteClient::from_service(service);

    assert_eq!(client.query_raw([7]).await.unwrap(), vec![7]);
}

#[tokio::test]
async fn query_typed_decodes_success_response_and_rejects_unexpected_type() {
    let info = crate::tl::response::MasterchainInfo {
        last: test_block_id(),
        state_root_hash: Int256([5; 32]),
        init: test_zero_state_id(),
    };
    let mut client = response_client(Response::MasterchainInfo(info.clone()));
    let decoded: crate::tl::response::MasterchainInfo = client
        .query_typed(Request::GetMasterchainInfo)
        .await
        .unwrap();
    assert_eq!(decoded, info);

    let mut client = response_client(Response::CurrentTime(crate::tl::response::CurrentTime {
        now: 123,
    }));
    let error = client
        .query_typed::<crate::tl::response::MasterchainInfo>(Request::GetMasterchainInfo)
        .await
        .unwrap_err();
    assert!(matches!(error, LiteError::UnexpectedMessage));
}

#[tokio::test]
async fn lookup_block_builds_request_and_decodes_block_header_response() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let block = BlockId {
        workchain: -1,
        shard: i64::MIN,
        seqno: 7,
    };
    let response = crate::tl::response::BlockHeader {
        id: test_block_id(),
        mode: (),
        with_state_update: Some(()),
        with_value_flow: None,
        with_extra: Some(()),
        with_shard_hashes: None,
        with_prev_blk_signatures: Some(()),
        header_proof: vec![1, 2, 3],
    };
    let mut client = request_response_client(
        Arc::clone(&requests),
        Response::BlockHeader(response.clone()),
    );

    let decoded = client
        .lookup_block(
            (),
            block.clone(),
            Some(()),
            Some(11),
            Some(22),
            true,
            false,
            true,
            false,
            true,
        )
        .await
        .unwrap();

    assert_eq!(decoded, response);
    let request: Request = tl_proto::deserialize(&requests.lock().await[0]).unwrap();
    assert_eq!(
        request,
        Request::LookupBlock(crate::tl::request::LookupBlock {
            mode: (),
            id: block,
            seqno: Some(()),
            lt: Some(11),
            utime: Some(22),
            with_state_update: Some(()),
            with_value_flow: None,
            with_extra: Some(()),
            with_shard_hashes: None,
            with_prev_blk_signatures: Some(()),
        })
    );
}

#[tokio::test]
async fn lookup_block_with_proof_builds_request_and_decodes_result() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let id = BlockId {
        workchain: 0,
        shard: 1,
        seqno: 9,
    };
    let mc_block_id = test_block_id();
    let response = crate::tl::response::LookupBlockResult {
        id: test_block_id(),
        mode: (),
        mc_block_id: mc_block_id.clone(),
        client_mc_state_proof: vec![1],
        mc_block_proof: vec![2],
        shard_links: Vec::new(),
        header: vec![3],
        prev_header: vec![4],
    };
    let mut client = request_response_client(
        Arc::clone(&requests),
        Response::LookupBlockResult(response.clone()),
    );

    let decoded = client
        .lookup_block_with_proof((), id.clone(), mc_block_id.clone(), Some(33), Some(44))
        .await
        .unwrap();

    assert_eq!(decoded, response);
    let request: Request = tl_proto::deserialize(&requests.lock().await[0]).unwrap();
    assert_eq!(
        request,
        Request::LookupBlockWithProof(crate::tl::request::LookupBlockWithProof {
            mode: (),
            id,
            mc_block_id,
            lt: Some(33),
            utime: Some(44),
        })
    );
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
async fn typed_boc_helpers_return_decode_errors_for_malformed_payloads() {
    let mut client = response_client(Response::BlockData(crate::tl::response::BlockData {
        id: test_block_id(),
        data: vec![0, 1, 2],
    }));
    let error = client
        .raw_get_block_data(test_block_id())
        .await
        .unwrap_err();
    assert!(matches!(error, LiteError::TlError(_)));

    let mut client = response_client(Response::LibraryResult(
        crate::tl::response::LibraryResult {
            result: vec![LibraryEntry {
                hash: Int256([9; 32]),
                data: vec![0, 1, 2],
            }],
        },
    ));
    let error = client
        .get_libraries_typed(vec![Int256([9; 32])])
        .await
        .unwrap_err();
    assert!(matches!(error, LiteError::TlError(_)));
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
