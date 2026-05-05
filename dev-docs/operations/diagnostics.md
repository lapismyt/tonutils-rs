# Diagnostics And Observability

The SDK should provide useful diagnostics without forcing logging or metrics dependencies on all users.

## Logging

Low-level logs should be structured enough to debug:

- peer address,
- ADNL connection phase,
- TL constructor name or id,
- request latency,
- liteserver error code,
- balancer peer state transition.

Avoid logging:

- private keys,
- shared secrets,
- AES keys and nonces,
- full message bodies by default.

## Metrics

Optional metrics can include:

- requests started,
- requests succeeded,
- requests failed by family,
- ADNL reconnect count,
- peer state counts,
- latency histograms,
- bytes sent and received.

Metrics must be behind a feature gate.

## Error Context

Errors should carry enough context to answer:

- which subsystem failed,
- whether retry is useful,
- whether data is untrusted,
- which peer produced the failure.

## Missing Work

- Feature-gated metrics facade.
- Structured debug formatting for TL objects.
- Redaction helpers for sensitive protocol material.
