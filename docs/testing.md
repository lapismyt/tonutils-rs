# Testing

The repository keeps deterministic checks separate from live-network workflows.
Local checks should not require secrets, public liteserver availability, or
external network access.

Audience: contributors adding code, fixtures, examples, or documentation that
could affect compile targets. Use this guide together with `AGENTS.md` for
change workflow and `dev-docs/testing/fixtures.md` for fixture metadata rules.

## Local Checks

Run the minimum verification before merging SDK changes:

```bash
cargo check
cargo test --lib
```

When examples or feature documentation change, also compile the examples with
the narrowest feature set they require:

```bash
cargo check --examples --all-features
```

Feature-gated work should add the matching checks, such as
`cargo check --no-default-features`, `cargo check --all-features`, or targeted
feature combinations.

Benchmark harnesses are compile-checked without running measurements with:

```bash
cargo bench --no-run
```

The deterministic offline harnesses can be run directly when measuring a local
change:

```bash
cargo bench --bench wallet
cargo bench --bench protocol
```

## Examples

Examples are written so they compile without live inputs. Runtime examples read
environment variables, but live-network examples now default to public mainnet
config download when `TON_GLOBAL_CONFIG_JSON` is absent. Offline examples use
deterministic fixtures when possible.

Current live example variables:

- `TON_NETWORK`: `mainnet` or `testnet`, defaulting to `mainnet`.
- `TON_GLOBAL_CONFIG_JSON`: full TON global config JSON, overriding public
  config download.
- `TON_LS_INDEX`: liteserver index for single-peer examples, defaulting to `0`.
- `TON_CONTRACT_ADDRESS`: account address for contract examples. Mainnet
  get-method examples default to
  `UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP`; testnet get-method
  examples require an explicit address and exit successfully when it is absent.
- `TON_GET_METHOD`: optional get-method name, defaulting to `seqno`.
- `TON_LITEAPI_REQUEST_HEX`: serialized LiteAPI request bytes for raw queries,
  defaulting to serialized `liteServer.getTime`.

Useful live smoke checks:

```bash
cargo run -F full --example network_config
cargo run -F full --example liteclient_masterchain_info
TON_NETWORK=testnet cargo run -F full --example liteclient_masterchain_info
cargo run -F full --example contract_get_method
```

## Live-Network Tests

Live-network tests should be ignored by default and documented with exact
inputs. They must not depend on repository-local secrets or a specific public
liteserver staying healthy.

Use live tests for compatibility evidence, not as the only coverage for a
protocol path. Add fixture-backed tests for TL bytes, BoC payloads, stack
values, proofs, and transport frames whenever the behavior can be captured.

## Fixtures

Binary fixtures should include source notes: upstream schema revision, command
used to capture the data, network, block id, account address, and whether the
data came from a live liteserver or from an upstream repository. Fixtures for
malformed inputs should explain the exact invariant being violated.

Known fixture gaps remain for official HashmapE encodings, cell hashes, block
proofs, account-state proofs, and successful live get-method stack shapes.
