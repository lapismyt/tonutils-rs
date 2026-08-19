# LiteClient Workflow Coverage

This page records the Phase 2 LiteClient acceptance boundary. The workflow is
offline-first: normal tests use service fakes and deterministic BoC fixtures;
network smoke tests remain explicitly ignored and are also exercised by the CI
workflow against a public TON liteserver configuration.

## Offline Acceptance

`tonutils-liteclient` tests cover:

- raw request and response byte preservation;
- typed LiteAPI decoding for masterchain info, time, version, block, account,
  transaction, config, and run-method responses;
- malformed TL response handling and unexpected typed response rejection;
- request-call timeout mapping to `LiteError::Timeout`;
- local block/account/proof BoC decode failures and proof/root mismatch errors;
- configuration-selection errors before network I/O for empty configs and invalid
  liteserver indexes;
- LiteBalancer delegation parity for masterchain info, time, and version, plus
  retry, archival routing, and non-retryable error behavior.

The checked fixture manifest is `fixtures/liteclient/workflow_payloads.json`.
Its metadata records the schema revision, source, capture date, payload role,
and expected local behavior. Synthetic fixtures are compatibility evidence for
local codecs only; they do not establish proof trust.

## Opt-In Live Smoke Tests

The ignored integration tests in `tests/live_workflows.rs` exercise:

- `getMasterchainInfo`;
- `getVersion` and `getTime`;
- `run_get_method` for `seqno` on the configured public contract.

They use a public TON global config supplied through `TON_GLOBAL_CONFIG_JSON`.
`TON_LS_INDEX` selects the liteserver, defaulting to `0`; the get-method test
uses `TON_CONTRACT_ADDRESS` or the stable mainnet example contract. If the
config variable is absent, the test reports a clear skip and succeeds without
network access.

Run explicitly with:

```text
TON_GLOBAL_CONFIG_JSON='<global-config-json>' cargo test -p tonutils-liteclient --test live_workflows -- --ignored --nocapture
```

The `live-tests` GitHub Actions job runs the same command on pushes to `main`,
pull requests targeting `main`, and manual `workflow_dispatch` runs. It loads
`https://ton.org/global.config.json` at runtime, passes the JSON through
`GITHUB_ENV`, and does not require repository secrets. The job uses the
existing default `TON_LS_INDEX=0` and the stable mainnet contract configured by
`live_workflows.rs` unless an environment override is added later.

Because the tests depend on public internet access and an available public
liteserver, a run can fail transiently because of DNS, HTTP, routing, endpoint
availability, or liteserver load. Such failures do not necessarily indicate a
LiteClient regression; inspect the job logs and rerun the workflow when the
public service is temporarily unavailable.

Live captures are not required in CI. Captured block/account/proof payloads
from a public liteserver remain a follow-up evidence tier and must record the
endpoint or upstream source, capture date, schema revision, block/account
identifiers, and expected root/file hashes before being treated as stronger
wire-compatibility evidence.
