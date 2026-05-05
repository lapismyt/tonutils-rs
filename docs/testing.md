# Testing

The repository keeps deterministic checks separate from live-network workflows.
Local checks should not require secrets, public liteserver availability, or
external network access.

## Local Checks

Run the minimum verification before merging SDK changes:

```bash
cargo check
cargo test --lib
```

When examples or feature documentation change, also compile the examples with
the narrowest feature set they require:

```bash
cargo check --examples --features network-config
```

Feature-gated work should add the matching checks, such as
`cargo check --no-default-features`, `cargo check --all-features`, or targeted
feature combinations.

## Examples

Examples are written so they compile without live inputs. Runtime examples read
environment variables and exit successfully with a short stderr message when
the input is missing.

Current live example variables:

- `TON_GLOBAL_CONFIG_JSON`: full TON global config JSON.
- `TON_CONTRACT_ADDRESS`: account address for contract examples.
- `TON_GET_METHOD`: optional get-method name, defaulting to `seqno`.
- `TON_LITEAPI_REQUEST_HEX`: serialized LiteAPI request bytes for raw queries.

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
