#[cfg(test)]
mod tests {
    use super::*;
    use crate::adnl::helper_types::AdnlError;
    use crate::tl::request::Request;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;
    use tower::service_fn;

    fn mock_client() -> LiteClient {
        LiteClient::from_service(service_fn(
            |request: crate::tl::request::RawWrappedRequest| async move {
                Ok::<_, LiteError>(request.request)
            },
        ))
    }

    fn send_message_client(calls: Arc<Mutex<usize>>) -> LiteClient {
        LiteClient::from_service(service_fn(
            move |_request: crate::tl::request::RawWrappedRequest| {
                let calls = Arc::clone(&calls);
                async move {
                    *calls.lock().await += 1;
                    Ok::<_, LiteError>(tl_proto::serialize(
                        crate::tl::response::Response::SendMsgStatus(
                            crate::tl::response::SendMsgStatus { status: 1 },
                        ),
                    ))
                }
            },
        ))
    }

    fn queued_response_client(
        calls: Arc<Mutex<usize>>,
        outcomes: Vec<std::result::Result<crate::tl::response::Response, LiteError>>,
    ) -> LiteClient {
        let outcomes = Arc::new(Mutex::new(std::collections::VecDeque::from(outcomes)));
        LiteClient::from_service(service_fn(
            move |_request: crate::tl::request::RawWrappedRequest| {
                let calls = Arc::clone(&calls);
                let outcomes = Arc::clone(&outcomes);
                async move {
                    *calls.lock().await += 1;
                    match outcomes.lock().await.pop_front() {
                        Some(Ok(response)) => Ok::<_, LiteError>(tl_proto::serialize(response)),
                        Some(Err(error)) => Err(error),
                        None => Err(LiteError::UnexpectedMessage),
                    }
                }
            },
        ))
    }

    fn recording_response_client(
        requests: Arc<Mutex<Vec<Request>>>,
        response: crate::tl::response::Response,
    ) -> LiteClient {
        let response = tl_proto::serialize(response);
        LiteClient::from_service(service_fn(
            move |request: crate::tl::request::RawWrappedRequest| {
                let requests = Arc::clone(&requests);
                let response = response.clone();
                async move {
                    let request = tl_proto::deserialize(&request.request).map_err(|error| {
                        LiteError::TlError(crate::tl::TlError::ParseError(error.to_string()))
                    })?;
                    requests.lock().await.push(request);
                    Ok::<_, LiteError>(response)
                }
            },
        ))
    }

    fn test_block_id(seqno: i32) -> BlockIdExt {
        BlockIdExt {
            workchain: -1,
            shard: i64::MIN,
            seqno,
            root_hash: Int256([seqno as u8; 32]),
            file_hash: Int256([(seqno + 1) as u8; 32]),
        }
    }

    fn block_header_response(seqno: i32) -> crate::tl::response::Response {
        crate::tl::response::Response::BlockHeader(crate::tl::response::BlockHeader {
            id: test_block_id(seqno),
            mode: (),
            with_state_update: None,
            with_value_flow: None,
            with_extra: None,
            with_shard_hashes: None,
            with_prev_blk_signatures: None,
            header_proof: Vec::new(),
        })
    }

    #[test]
    fn test_balancer_error_display() {
        let err = BalancerError::NoAlivePeers;
        assert_eq!(err.to_string(), "No alive peers available");

        let err = BalancerError::NoArchivePeers;
        assert_eq!(err.to_string(), "No alive archive peers available");

        let err = BalancerError::Timeout;
        assert_eq!(err.to_string(), "Timeout error");
    }

    #[test]
    fn test_peer_stats_default() {
        let stats = PeerStats::default();
        assert_eq!(stats.mc_block_seqno, 0);
        assert_eq!(stats.avg_response_time_ms, 0);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.current_requests, 0);
    }

    #[test]
    fn test_calc_new_average() {
        // First request
        let avg = LiteBalancer::calc_new_average(0, 0, 100);
        assert_eq!(avg, 100);

        // Second request
        let avg = LiteBalancer::calc_new_average(100, 1, 200);
        assert_eq!(avg, 150);

        // Third request
        let avg = LiteBalancer::calc_new_average(150, 2, 300);
        assert_eq!(avg, 200);

        // Multiple requests
        let avg = LiteBalancer::calc_new_average(200, 10, 100);
        assert_eq!(avg, (200 * 10 + 100) / 11);
    }

    #[test]
    fn archive_probe_seqno_is_positive_old_block_range() {
        for _ in 0..256 {
            let seqno = LiteBalancer::archive_probe_seqno();
            assert!((1..=1024).contains(&seqno));
        }
    }

    #[tokio::test]
    async fn test_balancer_initialization() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        assert_eq!(balancer.peers_num(), 0);
        assert_eq!(balancer.alive_peers_num().await, 0);
        assert_eq!(balancer.archival_peers_num().await, 0);
        assert!(!balancer.is_inited().await);
        assert_eq!(balancer.max_req_per_peer, 100);
        assert_eq!(balancer.max_retries, 1);
        assert_eq!(balancer.timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn test_balancer_configuration() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(5));

        balancer.max_req_per_peer = 50;
        balancer.max_retries = 3;

        assert_eq!(balancer.max_req_per_peer, 50);
        assert_eq!(balancer.max_retries, 3);
        assert_eq!(balancer.timeout, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn per_peer_rate_limit_is_attached_to_all_peers() {
        let peers = vec![mock_client(), mock_client()];
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10))
            .with_rate_limit_per_peer(RequestRateLimit::per_second(5).unwrap());

        assert!(balancer.peers.iter().all(LiteClient::has_rate_limiter));
    }

    #[tokio::test]
    async fn global_rate_limit_is_acquired_per_execute_request_attempt() {
        let mut balancer = LiteBalancer::new(Vec::new(), Duration::from_secs(10))
            .with_global_rate_limit(RequestRateLimit::with_burst(1, 1).unwrap());
        balancer.alive_peers.write().await.insert(0);

        let (peer_idx, start) = balancer.execute_request_for_test(false).await.unwrap();
        balancer.complete_request(peer_idx, start, true).await;

        let pending = tokio::time::timeout(
            Duration::from_millis(10),
            balancer.execute_request_for_test(false),
        )
        .await;
        assert!(pending.is_err());
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
    async fn send_message_global_rate_limit_counts_each_peer_attempt() {
        let calls = Arc::new(Mutex::new(0usize));
        let peers = vec![
            send_message_client(Arc::clone(&calls)),
            send_message_client(Arc::clone(&calls)),
        ];
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10))
            .with_global_rate_limit(RequestRateLimit::with_burst(1, 1).unwrap());
        balancer.alive_peers.write().await.insert(0);
        balancer.alive_peers.write().await.insert(1);

        let pending =
            tokio::time::timeout(Duration::from_millis(10), balancer.send_message(vec![1])).await;

        assert!(pending.is_err());
        assert_eq!(*calls.lock().await, 1);
    }

    #[tokio::test]
    async fn test_empty_balancer_choose_peer() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        let result = balancer.choose_peer(false).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BalancerError::NoAlivePeers));
    }

    #[tokio::test]
    async fn test_empty_balancer_choose_archive_peer() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        let result = balancer.choose_peer(true).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BalancerError::NoArchivePeers));
    }

    #[tokio::test]
    async fn test_consensus_block_empty() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        let consensus = balancer.find_consensus_block().await;
        assert_eq!(consensus, 0);
    }

    #[tokio::test]
    async fn test_consensus_block_calculation() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add some peer stats manually
        {
            let mut stats = balancer.peer_stats.write().await;

            // Add 5 peers with different seqnos
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 50,
                    total_requests: 10,
                    current_requests: 0,
                },
            );
            stats.insert(
                1,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 60,
                    total_requests: 8,
                    current_requests: 0,
                },
            );
            stats.insert(
                2,
                PeerStats {
                    mc_block_seqno: 99,
                    avg_response_time_ms: 40,
                    total_requests: 12,
                    current_requests: 0,
                },
            );
            stats.insert(
                3,
                PeerStats {
                    mc_block_seqno: 101,
                    avg_response_time_ms: 55,
                    total_requests: 9,
                    current_requests: 0,
                },
            );
            stats.insert(
                4,
                PeerStats {
                    mc_block_seqno: 98,
                    avg_response_time_ms: 70,
                    total_requests: 5,
                    current_requests: 0,
                },
            );
        }

        let consensus = balancer.find_consensus_block().await;
        // With 5 peers, 2/3 index = 3, sorted descending: [101, 100, 100, 99, 98]
        // consensus should be at index 3, which is 99
        assert_eq!(consensus, 99);
    }

    #[tokio::test]
    async fn test_build_priority_list_sorting() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add some alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }

        // Add peer stats with different characteristics
        {
            let mut stats = balancer.peer_stats.write().await;

            // Peer 0: high seqno, medium response time
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 50,
                    total_requests: 10,
                    current_requests: 0,
                },
            );

            // Peer 1: high seqno, low response time (should be first)
            stats.insert(
                1,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 30,
                    total_requests: 15,
                    current_requests: 0,
                },
            );

            // Peer 2: low seqno, low response time (should be last)
            stats.insert(
                2,
                PeerStats {
                    mc_block_seqno: 95,
                    avg_response_time_ms: 25,
                    total_requests: 20,
                    current_requests: 0,
                },
            );
        }

        let priority_list = balancer.build_priority_list(false).await;

        // Expected order: peer 1 (seqno 100, 30ms), peer 0 (seqno 100, 50ms), peer 2 (seqno 95, 25ms)
        assert_eq!(priority_list.len(), 3);
        assert_eq!(priority_list[0], 1); // Best peer
        assert_eq!(priority_list[1], 0);
        assert_eq!(priority_list[2], 2); // Worst peer (low seqno)
    }

    #[tokio::test]
    async fn test_update_average_request_time() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // First update
        balancer.update_average_request_time(0, 100).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 100);
            assert_eq!(peer_stats.total_requests, 1);
        }

        // Second update
        balancer.update_average_request_time(0, 200).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 150);
            assert_eq!(peer_stats.total_requests, 2);
        }

        // Third update
        balancer.update_average_request_time(0, 300).await;
        {
            let stats = balancer.peer_stats.read().await;
            let peer_stats = stats.get(&0).unwrap();
            assert_eq!(peer_stats.avg_response_time_ms, 200);
            assert_eq!(peer_stats.total_requests, 3);
        }
    }

    #[tokio::test]
    async fn test_delete_unsync_peers() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
            alive.insert(3);
        }

        // Add peer stats
        {
            let mut stats = balancer.peer_stats.write().await;
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    ..Default::default()
                },
            );
            stats.insert(
                1,
                PeerStats {
                    mc_block_seqno: 100,
                    ..Default::default()
                },
            );
            stats.insert(
                2,
                PeerStats {
                    mc_block_seqno: 98, // Out of sync
                    ..Default::default()
                },
            );
            stats.insert(
                3,
                PeerStats {
                    mc_block_seqno: 99,
                    ..Default::default()
                },
            );
        }

        balancer.delete_unsync_peers().await;

        let alive = balancer.alive_peers.read().await;
        // Consensus should be 99 or 100 depending on calculation
        // Peers with seqno >= consensus should remain
        assert!(alive.contains(&0));
        assert!(alive.contains(&1));
    }

    #[tokio::test]
    async fn test_choose_peer_with_load() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        balancer.max_req_per_peer = 5;

        // Add alive peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }

        // Add peer stats with different loads
        {
            let mut stats = balancer.peer_stats.write().await;

            // Peer 0: overloaded
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 50,
                    total_requests: 10,
                    current_requests: 10, // Over limit
                },
            );

            // Peer 1: available (should be chosen)
            stats.insert(
                1,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 60,
                    total_requests: 5,
                    current_requests: 2, // Under limit
                },
            );

            // Peer 2: available but slower
            stats.insert(
                2,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 80,
                    total_requests: 8,
                    current_requests: 3, // Under limit
                },
            );
        }

        let chosen = balancer.choose_peer(false).await.unwrap();
        // Should choose peer 1 (under limit and faster than peer 2)
        assert_eq!(chosen, 1);
    }

    #[tokio::test]
    async fn choose_peer_treats_limit_as_exclusive() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));
        balancer.max_req_per_peer = 5;

        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
        }

        {
            let mut stats = balancer.peer_stats.write().await;
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 10,
                    total_requests: 1,
                    current_requests: 5,
                },
            );
            stats.insert(
                1,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 20,
                    total_requests: 1,
                    current_requests: 4,
                },
            );
        }

        let chosen = balancer.choose_peer(false).await.unwrap();
        assert_eq!(chosen, 1);
    }

    #[tokio::test]
    async fn test_archival_peers_filtering() {
        let peers = Vec::new();
        let balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add some alive and archival peers
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
            alive.insert(1);
            alive.insert(2);
        }

        {
            let mut archival = balancer.archival_peers.write().await;
            archival.insert(1); // Only peer 1 is archival
        }

        // Build priority list for archival only
        let archival_list = balancer.build_priority_list(true).await;
        assert_eq!(archival_list.len(), 1);
        assert_eq!(archival_list[0], 1);

        // Build priority list for all
        let all_list = balancer.build_priority_list(false).await;
        assert_eq!(all_list.len(), 3);
    }

    #[tokio::test]
    async fn typed_helper_retries_adnl_errors_through_peer_selection_path() {
        let peer0_calls = Arc::new(Mutex::new(0usize));
        let peer1_calls = Arc::new(Mutex::new(0usize));
        let peers = vec![
            queued_response_client(
                Arc::clone(&peer0_calls),
                vec![Err(LiteError::AdnlError(AdnlError::EndOfStream))],
            ),
            queued_response_client(
                Arc::clone(&peer1_calls),
                vec![Ok(crate::tl::response::Response::LibraryResult(
                    crate::tl::response::LibraryResult {
                        result: vec![LibraryEntry {
                            hash: Int256([7; 32]),
                            data: Vec::new(),
                        }],
                    },
                ))],
            ),
        ];
        let mut balancer = LiteBalancer::new(peers, Duration::from_millis(50));
        balancer.max_retries = 2;
        balancer.alive_peers.write().await.extend([0, 1]);
        balancer.peer_stats.write().await.extend([
            (
                0,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 1,
                    ..Default::default()
                },
            ),
            (
                1,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 2,
                    ..Default::default()
                },
            ),
        ]);

        let result = balancer
            .get_libraries_typed(vec![Int256([7; 32])])
            .await
            .unwrap();

        assert!(matches!(result.get(&Int256([7; 32])), Some(None)));
        assert_eq!(*peer0_calls.lock().await, 1);
        assert_eq!(*peer1_calls.lock().await, 1);
        assert!(!balancer.alive_peers.read().await.contains(&0));
        assert!(balancer.alive_peers.read().await.contains(&1));
        assert_eq!(
            balancer.peer_states.read().await.get(&0),
            Some(&PeerState::Dead)
        );
        assert_eq!(
            balancer
                .peer_stats
                .read()
                .await
                .get(&0)
                .map(|stats| (stats.total_requests, stats.current_requests)),
            Some((1, 0))
        );
    }

    #[tokio::test]
    async fn lite_server_error_is_not_retried_but_updates_peer_failure_state() {
        let peer0_calls = Arc::new(Mutex::new(0usize));
        let peer1_calls = Arc::new(Mutex::new(0usize));
        let peers = vec![
            queued_response_client(
                Arc::clone(&peer0_calls),
                vec![Ok(crate::tl::response::Response::Error(
                    crate::tl::response::Error {
                        code: 400,
                        message: "bad request".into(),
                    },
                ))],
            ),
            queued_response_client(
                Arc::clone(&peer1_calls),
                vec![Ok(crate::tl::response::Response::CurrentTime(
                    crate::tl::response::CurrentTime { now: 1 },
                ))],
            ),
        ];
        let mut balancer = LiteBalancer::new(peers, Duration::from_millis(50));
        balancer.max_retries = 2;
        balancer.alive_peers.write().await.extend([0, 1]);
        balancer.peer_stats.write().await.extend([
            (
                0,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 1,
                    ..Default::default()
                },
            ),
            (
                1,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 2,
                    ..Default::default()
                },
            ),
        ]);

        let error = balancer.get_time().await.unwrap_err();

        assert!(matches!(
            error,
            BalancerError::LiteError(LiteError::ServerError(_))
        ));
        assert_eq!(*peer0_calls.lock().await, 1);
        assert_eq!(*peer1_calls.lock().await, 0);
        assert!(!balancer.alive_peers.read().await.contains(&0));
        assert_eq!(
            balancer.peer_states.read().await.get(&0),
            Some(&PeerState::Dead)
        );
        assert_eq!(
            balancer
                .peer_stats
                .read()
                .await
                .get(&0)
                .map(|stats| (stats.total_requests, stats.current_requests)),
            Some((1, 0))
        );
    }

    #[tokio::test]
    async fn local_typed_decode_error_is_not_retried() {
        let peer0_calls = Arc::new(Mutex::new(0usize));
        let peer1_calls = Arc::new(Mutex::new(0usize));
        let peers = vec![
            queued_response_client(
                Arc::clone(&peer0_calls),
                vec![Ok(crate::tl::response::Response::LibraryResult(
                    crate::tl::response::LibraryResult {
                        result: vec![LibraryEntry {
                            hash: Int256([8; 32]),
                            data: vec![0, 1, 2],
                        }],
                    },
                ))],
            ),
            queued_response_client(
                Arc::clone(&peer1_calls),
                vec![Ok(crate::tl::response::Response::LibraryResult(
                    crate::tl::response::LibraryResult { result: Vec::new() },
                ))],
            ),
        ];
        let mut balancer = LiteBalancer::new(peers, Duration::from_millis(50));
        balancer.max_retries = 2;
        balancer.alive_peers.write().await.extend([0, 1]);
        balancer.peer_stats.write().await.extend([
            (
                0,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 1,
                    ..Default::default()
                },
            ),
            (
                1,
                PeerStats {
                    mc_block_seqno: 10,
                    avg_response_time_ms: 2,
                    ..Default::default()
                },
            ),
        ]);

        let error = balancer
            .get_libraries_typed(vec![Int256([8; 32])])
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            BalancerError::LiteError(LiteError::TlError(_))
        ));
        assert_eq!(*peer0_calls.lock().await, 1);
        assert_eq!(*peer1_calls.lock().await, 0);
        assert!(!balancer.alive_peers.read().await.contains(&0));
    }

    #[tokio::test]
    async fn archive_probe_records_only_peers_that_accept_old_block_lookup() {
        let peer1_requests = Arc::new(Mutex::new(Vec::new()));
        let peers = vec![
            queued_response_client(
                Arc::new(Mutex::new(0)),
                vec![Ok(crate::tl::response::Response::Error(
                    crate::tl::response::Error {
                        code: 404,
                        message: "not archive".into(),
                    },
                ))],
            ),
            recording_response_client(Arc::clone(&peer1_requests), block_header_response(1)),
        ];
        let mut balancer = LiteBalancer::new(peers, Duration::from_millis(50));
        balancer.alive_peers.write().await.extend([0, 1]);

        balancer.find_archives().await;

        assert_eq!(*balancer.archival_peers.read().await, HashSet::from([1]));
        let requests = peer1_requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert!(matches!(requests[0], Request::LookupBlock(_)));
    }

    #[tokio::test]
    async fn test_balancer_close_all() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Manually set inited to true
        *balancer.inited.write().await = true;

        // Start a dummy health checker
        let handle = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(1000)).await;
        });
        *balancer.checker_handle.write().await = Some(handle);

        assert!(balancer.is_inited().await);

        // Close all
        balancer.close_all().await.unwrap();

        assert!(!balancer.is_inited().await);
        assert!(balancer.checker_handle.read().await.is_none());
    }

    #[test]
    fn test_balancer_error_from_lite_error() {
        let lite_err = LiteError::UnexpectedMessage;
        let balancer_err: BalancerError = lite_err.into();

        assert!(matches!(balancer_err, BalancerError::LiteError(_)));
    }

    #[tokio::test]
    async fn test_execute_request_increments_counter() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add an alive peer
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
        }

        // Execute request
        let (_peer_idx, _start) = balancer.execute_request::<()>(false).await.unwrap();

        // Check that counter was incremented
        let stats = balancer.peer_stats.read().await;
        let peer_stats = stats.get(&0).unwrap();
        assert_eq!(peer_stats.current_requests, 1);
    }

    #[tokio::test]
    async fn test_complete_request_decrements_counter() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add peer with active request
        {
            let mut stats = balancer.peer_stats.write().await;
            stats.insert(
                0,
                PeerStats {
                    mc_block_seqno: 100,
                    avg_response_time_ms: 50,
                    total_requests: 5,
                    current_requests: 3,
                },
            );
        }

        let start = Instant::now();
        balancer.complete_request(0, start, true).await;

        // Check that counter was decremented
        let stats = balancer.peer_stats.read().await;
        let peer_stats = stats.get(&0).unwrap();
        assert_eq!(peer_stats.current_requests, 2);
    }

    #[tokio::test]
    async fn test_complete_request_removes_failed_peer() {
        let peers = Vec::new();
        let mut balancer = LiteBalancer::new(peers, Duration::from_secs(10));

        // Add alive peer
        {
            let mut alive = balancer.alive_peers.write().await;
            alive.insert(0);
        }

        let start = Instant::now();
        balancer.complete_request(0, start, false).await;

        // Check that peer was removed from alive set
        let alive = balancer.alive_peers.read().await;
        assert!(!alive.contains(&0));
    }
}
