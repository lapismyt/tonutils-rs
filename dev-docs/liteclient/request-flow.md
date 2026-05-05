# LiteClient Request Flow

LiteClient executes LiteAPI calls over ADNL TCP.

## Flow

1. Establish ADNL TCP connection.
2. Acquire the optional local request-rate limiter.
3. Wrap LiteAPI request into `WrappedRequest`.
4. Serialize into `liteServer.query`.
5. Serialize into `adnl.message.query`.
6. Send through multiplexed ADNL stream.
7. Receive `adnl.message.answer`.
8. Decode answer bytes as `Response`.
9. Convert response to typed output or return server error.

## Query Ids

ADNL queries use `query_id:int256`. The current peer layer assigns random ids for multiplexing. Responses must match the original query id.

## Wait Seqno

`waitMasterchainSeqno` is a prefix. It delays request execution until the liteserver catches up or times out.

The local rate limiter runs before `waitMasterchainSeqno` is consumed. If a
task is cancelled while waiting for a limiter token, the pending seqno remains
attached to the next request.

## Rate Limiting

`RequestRateLimit` uses a local token bucket with whole-request `rps` and
`burst` values. It is not encoded into LiteAPI. When exhausted, `query_raw`
waits asynchronously and then sends the request, so typed helpers inherit the
same behavior through the shared raw path.

## Raw Request Path

The crate should support a truly raw path:

- input: already serialized LiteAPI request bytes,
- output: raw response bytes,
- no attempt to decode request into known enum.

This is required for future schema compatibility.

## Typed Request Path

Typed methods should exist for stable functions. Each method should:

- build request struct,
- set flags explicitly,
- call shared executor,
- convert response with exact expected type.

## Missing Work

- Shared executor trait for LiteClient and LiteBalancer.
- Better timeout configuration.
- Live-network integration tests.
