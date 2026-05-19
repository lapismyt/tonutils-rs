use super::*;

use crate::adnl::helper_types::AdnlError;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::service_fn;

fn response_client(
    calls: Arc<Mutex<usize>>,
    response: std::result::Result<crate::tl::response::Response, LiteError>,
) -> LiteClient {
    let response = Arc::new(Mutex::new(Some(response)));
    LiteClient::from_service(service_fn(
        move |_request: crate::tl::request::RawWrappedRequest| {
            let calls = Arc::clone(&calls);
            let response = Arc::clone(&response);
            async move {
                *calls.lock().await += 1;
                match response
                    .lock()
                    .await
                    .take()
                    .unwrap_or(Err(LiteError::UnexpectedMessage))
                {
                    Ok(response) => Ok::<_, LiteError>(tl_proto::serialize(response)),
                    Err(error) => Err(error),
                }
            }
        },
    ))
}

#[tokio::test]
async fn retryable_failures_transition_healthy_to_suspect_to_dead() {
    let mut balancer = LiteBalancer::new(Vec::new(), Duration::from_millis(25));
    balancer.alive_peers.write().await.insert(0);
    balancer
        .peer_states
        .write()
        .await
        .insert(0, PeerState::Healthy);

    balancer.complete_request(0, Instant::now(), false).await;
    assert_eq!(
        balancer.peer_states.read().await.get(&0),
        Some(&PeerState::Suspect)
    );
    assert!(balancer.alive_peers.read().await.contains(&0));

    balancer.complete_request(0, Instant::now(), false).await;
    assert_eq!(
        balancer.peer_states.read().await.get(&0),
        Some(&PeerState::Dead)
    );
    assert!(!balancer.alive_peers.read().await.contains(&0));
}

#[tokio::test]
async fn successful_request_resets_failure_count_and_marks_healthy() {
    let mut balancer = LiteBalancer::new(Vec::new(), Duration::from_millis(25));
    balancer.complete_request(0, Instant::now(), false).await;
    balancer.complete_request(0, Instant::now(), true).await;

    let stats = balancer.peer_stats.read().await;
    let stats = stats.get(&0).unwrap();
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.last_failure_kind, None);
    assert_eq!(
        balancer.peer_states.read().await.get(&0),
        Some(&PeerState::Healthy)
    );
}

#[tokio::test]
async fn non_retryable_errors_do_not_mark_peer_dead() {
    let mut balancer = LiteBalancer::new(Vec::new(), Duration::from_millis(25));
    balancer.alive_peers.write().await.insert(0);
    balancer
        .peer_states
        .write()
        .await
        .insert(0, PeerState::Healthy);
    let error = LiteError::TlError(crate::tl::TlError::ParseError("bad boc".into()));

    balancer
        .complete_request_error(0, Instant::now(), &error)
        .await;

    assert!(balancer.alive_peers.read().await.contains(&0));
    assert_eq!(
        balancer.peer_states.read().await.get(&0),
        Some(&PeerState::Healthy)
    );
}

#[tokio::test]
async fn in_flight_counters_decrement_on_success_timeout_and_error() {
    let mut balancer = LiteBalancer::new(Vec::new(), Duration::from_millis(25));
    {
        let mut stats = balancer.peer_stats.write().await;
        stats.insert(
            0,
            PeerStats {
                current_requests: 3,
                ..Default::default()
            },
        );
    }

    balancer.complete_request(0, Instant::now(), true).await;
    assert_eq!(
        balancer
            .peer_stats
            .read()
            .await
            .get(&0)
            .map(|stats| stats.current_requests),
        Some(2)
    );

    let timeout = LiteError::Timeout {
        operation: "request_call",
        timeout: Duration::from_millis(25),
    };
    balancer
        .complete_request_error(0, Instant::now(), &timeout)
        .await;
    assert_eq!(
        balancer
            .peer_stats
            .read()
            .await
            .get(&0)
            .map(|stats| stats.current_requests),
        Some(1)
    );

    let error = LiteError::UnexpectedMessage;
    balancer
        .complete_request_error(0, Instant::now(), &error)
        .await;
    assert_eq!(
        balancer
            .peer_stats
            .read()
            .await
            .get(&0)
            .map(|stats| stats.current_requests),
        Some(0)
    );
}

#[tokio::test]
async fn selection_prefers_latency_freshness_and_lower_in_flight_count() {
    let balancer = LiteBalancer::new(Vec::new(), Duration::from_millis(100));
    balancer.alive_peers.write().await.extend([0, 1, 2]);
    balancer.peer_stats.write().await.extend([
        (
            0,
            PeerStats {
                ewma_latency_ms: Some(10),
                last_observed_seqno: 100,
                current_requests: 4,
                ..Default::default()
            },
        ),
        (
            1,
            PeerStats {
                ewma_latency_ms: Some(15),
                last_observed_seqno: 100,
                current_requests: 0,
                ..Default::default()
            },
        ),
        (
            2,
            PeerStats {
                ewma_latency_ms: Some(1),
                last_observed_seqno: 99,
                current_requests: 0,
                ..Default::default()
            },
        ),
    ]);

    assert_eq!(balancer.build_priority_list(false).await, vec![1, 0, 2]);
}

#[tokio::test]
async fn send_message_attempts_distinct_peers_and_returns_first_success() {
    let peer0_calls = Arc::new(Mutex::new(0usize));
    let peer1_calls = Arc::new(Mutex::new(0usize));
    let peers = vec![
        response_client(
            Arc::clone(&peer0_calls),
            Err(LiteError::AdnlError(AdnlError::EndOfStream)),
        ),
        response_client(
            Arc::clone(&peer1_calls),
            Ok(crate::tl::response::Response::SendMsgStatus(
                crate::tl::response::SendMsgStatus { status: 1 },
            )),
        ),
    ];
    let mut balancer = LiteBalancer::new(peers, Duration::from_millis(25));
    balancer.max_retries = 2;
    balancer.alive_peers.write().await.extend([0, 1]);
    balancer.peer_stats.write().await.extend([
        (
            0,
            PeerStats {
                ewma_latency_ms: Some(1),
                last_observed_seqno: 1,
                ..Default::default()
            },
        ),
        (
            1,
            PeerStats {
                ewma_latency_ms: Some(2),
                last_observed_seqno: 1,
                ..Default::default()
            },
        ),
    ]);

    assert_eq!(balancer.send_message(vec![1, 2, 3]).await.unwrap(), 1);
    assert_eq!(*peer0_calls.lock().await, 1);
    assert_eq!(*peer1_calls.lock().await, 1);
}
