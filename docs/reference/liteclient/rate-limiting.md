# LiteClient Rate Limiting

## Purpose And Scope

LiteAPI providers may enforce rented-liteserver quotas such as a fixed number
of requests per second. The crate models these quotas as local throttling, not
as LiteAPI protocol messages. `LiteClient` can throttle one connection, and
`LiteBalancer` can throttle either every owned peer or the balancer's total
outgoing request attempts.

## Data Model

`RequestRateLimit { rps, burst }` defines a token bucket:

- `rps` is the steady-state refill rate in whole requests per second.
- `burst` is the maximum number of tokens and the number of immediate requests
  available after constructing the limiter.
- `rps == 0` and `burst == 0` are invalid.

The limiter has no wire format. It runs before a serialized LiteAPI request is
sent through `liteServer.query`.

## Invariants And Edge Cases

- Throttling waits asynchronously instead of returning a rate-limit error.
- Default clients and balancers are unlimited.
- A cancelled wait does not consume a `waitMasterchainSeqno` prefix because the
  limiter is acquired before `LiteClient::query_raw` takes the pending seqno.
- `LiteBalancer` global limiting counts actual attempts, including retries and
  each `send_message` peer attempt.
- Per-peer limiting is independent per `LiteClient`; it does not cap aggregate
  balancer throughput unless every peer has the same cap and peer selection is
  balanced.

## Current Crate Mapping

- `src/liteclient/rate_limit.rs` implements `RequestRateLimit`, validation, the
  async `RateLimiter`, and deterministic token-bucket tests.
- `src/liteclient/client.rs` stores an optional limiter and acquires it at the
  start of `query_raw`.
- `src/liteclient/balancer.rs` stores an optional global limiter and applies it
  in the shared `execute_request` path.
- `src/cli/mod.rs` maps `--rps` to per-liteserver limits and `--global-rps` to
  balancer-wide attempt limits.

## Missing Work

- Live validation against tonconsole-style rented liteserver credentials.
- Optional metrics for limiter wait time and throttled attempt counts.
- Broader integration tests for retry-heavy balancer flows.
