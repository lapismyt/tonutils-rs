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

CLI network commands add operation context before returning errors. High-level
commands include the target address, block, method, or backend mode where that
context is available. `LiteError::TlError` displays the inner parse error, and
liteserver errors display both server code and message.

## CLI Decode Policy

The CLI keeps LiteAPI TL response parsing strict: malformed or unexpected
LiteAPI responses fail the command. TL-B and BoC decoding of embedded account
state, proofs, and result payloads is best-effort for high-level inspection
commands. When a nested decode is unsupported or malformed, human output omits
raw bytes and reports byte lengths, root hashes for successfully decoded BoCs,
and `decode_error` lines. JSON and pretty JSON include the same `decode_errors`
array plus the structured fields that decoded successfully.

Structured command data is written to stdout. Connection warnings, peer startup
failures, and diagnostics are written to stderr.

## Live Smoke Coverage

On 2026-05-09, the account CLI command was smoke-tested against a live
LiteServer response:

```bash
cargo run -F full -- account UQA_rW3Zvza4OcuW0yh4vH-cno3X0IcABYAX3whMjO5BSsQn
```

The run verified structural diagnostics for multi-root account proof BoCs. The
output reported `shard_proof_root_count: 2`, two `shard_proof_root_hash` lines,
`proof_root_count: 2`, and two `proof_root_hash` lines, with no proof-related
`decode_error`. This is evidence for CLI inspection behavior only; it is not
trustless account-state proof verification.

## Missing Work

- Feature-gated metrics facade.
- Structured debug formatting for TL objects.
- Stable JSON error objects for CLI failures.
- Redaction helpers for sensitive protocol material.
