# CLI

The CLI is designed for shell scripts. Structured command results are written to
stdout. Diagnostics and connection warnings are written to stderr.

## Global Options

- `--network mainnet|testnet`: public config to download when no config is supplied.
- `--config <path>`: read TON global config JSON from a file.
- `--config-json <json>`: read TON global config JSON from an argument.
- `--output human|json|pretty-json|raw|hex|base64`: select stdout format.
- `--rps <N>`: throttle each selected liteserver to `N` requests per second.
- `--global-rps <N>`: throttle total LiteBalancer request attempts to `N`
  requests per second.

`--config` and `--config-json` are mutually exclusive. If neither is provided,
the CLI downloads the selected public config when the command needs a network
connection.

## LiteClient Commands

```bash
tonutils --output json liteclient masterchain-info --ls-index 0
tonutils --rps 5 --output json liteclient masterchain-info --ls-index 0
tonutils --output json liteclient version --ls-index 0
tonutils --output json liteclient time --ls-index 0
tonutils --output hex liteclient raw-query --ls-index 0 --hex '<request>'
tonutils --output json liteclient run-get-method --ls-index 0 --address '<addr>' --method seqno
```

Raw query input can be supplied with `--hex`, `--base64`, `--file`, or `--stdin`.

## Contract Commands

Contract commands use the high-level contract API and the latest masterchain
block reported by the selected liteserver.

```bash
tonutils --output json contract state --ls-index 0 --address '<addr>'
tonutils --output json contract run-get-method --ls-index 0 --address '<addr>' --method seqno
tonutils --output json contract run-get-method --ls-index 0 --address '<addr>' --method-id 85143
```

JSON state output includes the masterchain block id, shard block id, proof byte
lengths, and raw state bytes as hex and base64. JSON get-method output includes
the execution block ids, exit code, proof byte lengths, raw result BoC, and a
decoded stack when the current stack decoder supports the returned shape.

## Balancer Commands

```bash
tonutils --output json balancer status --num-servers 3
tonutils --rps 5 --global-rps 12 --output json balancer masterchain-info --num-servers 3
```

Balancer commands construct multiple LiteClient peers from the selected config.
They inherit the prototype balancer limits described in
[LiteBalancer](balancer.md).

`--rps` applies to every peer. `--global-rps` applies only to balancer commands
and counts retries and multi-peer message-send attempts as separate requests.

## Exit Behavior

Successful commands exit with code `0`. Command line parsing errors, network
errors, LiteAPI errors, and invalid input return nonzero exit codes through the
standard Rust error path. Structured command data is written to stdout; human
diagnostics and connection warnings should be treated as stderr.
