# Benchmarks

Performance must be measured before optimizing.

## Benchmark Targets

- ADNL frame encode/decode.
- TL request/response encode/decode.
- BoC serialization and deserialization.
- Cell representation hash.
- Builder bit writes.
- Slice bit reads.
- TVM stack encode/decode.
- Balancer peer selection.

## Metrics

Track:

- throughput,
- allocations,
- p50/p95/p99 latency for request paths,
- memory growth under long-running scans.

## Rules

- Keep benchmarks deterministic.
- Avoid live network in normal benchmarks.
- Add live latency benchmarks as explicit opt-in.
- Record input sizes.
