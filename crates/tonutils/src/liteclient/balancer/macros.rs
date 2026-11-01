macro_rules! balanced_call {
    ($self:ident, $response:ty, $only_archive:expr, |$client:ident| $call:expr) => {{
        for _attempt in 0..$self.max_retries {
            let (peer_idx, start) = $self.execute_request::<$response>($only_archive).await?;
            let result = {
                let $client = &mut $self.peers[peer_idx];
                $call.await
            };
            match result {
                Ok(response) => {
                    $self.complete_request(peer_idx, start, true).await;
                    return Ok(response);
                }
                Err(e) => {
                    let is_retryable = LiteBalancer::retryable_failure(&e).is_some();
                    $self.complete_request_error(peer_idx, start, &e).await;
                    if !is_retryable {
                        return Err(BalancerError::LiteError(e));
                    }
                }
            }
        }
        Err(BalancerError::Timeout)
    }};
}

pub(crate) use balanced_call;
