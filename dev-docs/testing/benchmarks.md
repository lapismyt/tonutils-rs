# Benchmarks

Performance must be measured before optimizing.

## Benchmark Targets

- Wallet and mnemonic hot paths:
  - mnemonic import and Ed25519 key derivation from a fixed accepted phrase,
  - TON default-seed PBKDF2 derivation,
  - TON seed-version validation derivation,
  - deterministic mnemonic generation with a seeded RNG,
  - cached Wallet V4R2 and V5R1 code cell access,
  - Wallet V4R2 and V5R1 address derivation,
  - Wallet V4R2 and V5R1 signed transfer external-message BoC construction,
  - standard wallet comment body construction.
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

## Commands

Run the deterministic wallet benchmark harness with:

```sh
cargo bench --bench wallet
```

All benchmarks in `benches/wallet.rs` are offline and use fixed mnemonics,
seeded RNGs, fixed addresses, and embedded wallet code cells.

Run deterministic protocol primitive benchmarks with:

```sh
cargo bench --bench protocol
```

`benches/protocol.rs` is offline and currently covers ADNL frame
encode/decode, TL request serialization/deserialization, cell hashing, BoC
serialization/deserialization, builder and slice bit operations, and nested TVM
stack BoC conversion. It uses fixed in-memory fixtures only. Balancer selection
benchmarks remain a separate follow-up because they need deterministic mock peer
state and request routing inputs.

When measuring CLI wallet commands, build and run the release binary. Timing
`cargo run` includes Cargo compilation and process setup noise:

```sh
cargo build --release --features cli
./target/release/tonutils-rs wallet address --help
```
