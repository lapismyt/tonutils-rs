# Testing Strategy

Testing must prove binary compatibility and prevent protocol drift.

## Local Tests

Must not require network. Cover:

- TL roundtrip,
- ADNL codec,
- TVM cell and BoC,
- stack encoding,
- balancer scoring,
- address parsing.

## Live Tests

Ignored by default. Cover:

- public config download,
- ADNL connect,
- `getMasterchainInfo`,
- `getVersion`,
- simple get-method.

## Negative Tests

Required for:

- malformed TL bytes,
- malformed BoC,
- invalid address checksum,
- ADNL tampering,
- stale balancer peers,
- invalid proofs.

## CI Commands

```bash
cargo fmt --check
cargo check --no-default-features
cargo check
cargo check --all-features
cargo test
cargo test --all-features
```
