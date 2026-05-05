# LiteBalancer

LiteBalancer routes LiteAPI requests across liteservers.

## Peer Metadata

A complete peer record should include:

- liteserver socket address,
- public key,
- connection state,
- last masterchain seqno,
- latency EWMA,
- in-flight request count,
- last success time,
- last failure time,
- archival capability,
- reconnect attempt count.

## Peer States

- `Healthy`: normal candidate.
- `Suspect`: degraded candidate.
- `Dead`: not eligible.
- `Recovering`: reconnect or probe in progress.

## Scoring

Priority should consider:

1. state,
2. archive requirement,
3. masterchain freshness,
4. EWMA latency,
5. in-flight load,
6. recent failures.

## Retry Policy

Retry on:

- connection reset,
- timeout,
- ADNL transport error,
- end of stream.

Do not blindly retry on:

- `liteServer.error`,
- contract execution exit code,
- malformed local request,
- proof verification failure.

## Rate Limiting

The balancer supports two independent limiter placements:

- per-peer limits stored on each owned `LiteClient`;
- a global limit stored on `LiteBalancer`.

The global limiter is acquired in the shared request execution path after a
peer has been selected and before in-flight counters are incremented. Retries
therefore consume additional tokens. `send_message` also consumes one global
token per peer attempt, not one token for the high-level method call.

## Missing Work

- Reconnect descriptors.
- EWMA implementation.
- State transition tests.
- Shared method dispatch with LiteClient.
- Live validation against rented liteserver quota behavior.
