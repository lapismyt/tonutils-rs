# CLI

The CLI is designed for shell scripts. Structured command results are written to
stdout. Diagnostics and connection warnings are written to stderr.

Audience: operators, examples, and tests that need reproducible command-line
access to LiteClient, LiteBalancer, contract, TVM, BoC, and schema workflows.
Prerequisites: build with the `cli` feature. Network commands need live network
access; `tvm` commands are offline.

## Global Options

- `--network mainnet|testnet`: public config to download when no config is supplied.
- `--config <path>`: read TON global config JSON from a file.
- `--config-json <json>`: read TON global config JSON from an argument.
- `--output human|json|pretty-json|raw|hex|base64`: select stdout format.
- `--rps <N>`: throttle each selected liteserver to `N` requests per second.
- `--global-rps <N>`: throttle total LiteBalancer request attempts to `N`
  requests per second.
- `--num-servers <N>`: number of liteservers for high-level balancer commands.
- `--single --ls-index <N>`: use one reproducible liteserver instead of the
  default high-level balancer.

`--config` and `--config-json` are mutually exclusive. If neither is provided,
the CLI downloads the selected public config when the command needs a network
connection.

## High-Level Network Commands

High-level commands use `LiteBalancer` by default, download the selected public
config when no config is supplied, and print compact human output. Use
`--output json` or `--output pretty-json` for complete structured output.

```bash
tonutils status
tonutils account UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP
tonutils --output json account UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP
tonutils call '<addr>' seqno
tonutils call '<addr>' 85143 --arg int:1 --arg null
tonutils transactions '<addr>' --count 20
tonutils block latest
tonutils block get '<wc:shard:seqno:root_hash:file_hash>'
tonutils config get
tonutils config get --params 0,17,34
```

`call` accepts stack arguments as `int:<decimal>`, `null`, `cell:<boc-hex>`,
and `slice:<boc-hex>`. Without `--arg`, it sends an empty stack.

`account` is best-effort after the strict LiteAPI response parse. If account
state TL-B decoding is incomplete, the command still prints byte lengths, root
hashes for successfully decoded BoCs, and `decode_errors`.

`transactions` needs the account's last transaction hash. Until verified
`ShardAccounts` proof-path extraction lands, the high-level command can report
the current account LT but may return no history with a `decode_error` explaining
that the hash is unavailable. Use `liteclient raw-get-transactions` or
`balancer raw-get-transactions` with an explicit `--lt` and `--hash` when those
values are known.

## Advanced LiteClient Commands

```bash
tonutils --output json liteclient masterchain-info --ls-index 0
tonutils --rps 5 --output json liteclient masterchain-info --ls-index 0
tonutils --output json liteclient version --ls-index 0
tonutils --output json liteclient time --ls-index 0
tonutils --output hex liteclient raw-query --ls-index 0 --hex '<request>'
tonutils --output json liteclient run-get-method --ls-index 0 --address '<addr>' --method seqno
```

Raw query input can be supplied with `--hex`, `--base64`, `--file`, or `--stdin`.

## Advanced Contract Commands

Contract commands use the high-level contract API and the latest masterchain
block reported by the selected liteserver.

```bash
tonutils --output json contract state --ls-index 0 --address '<addr>'
tonutils --output json contract run-get-method --ls-index 0 --address '<addr>' --method seqno
tonutils --output json contract run-get-method --ls-index 0 --address '<addr>' --method-id 85143
tonutils --output json contract run-abi-get-method --ls-index 0 --address '<addr>' --abi-file contract.abi.json --contract Wallet --method seqno --arg 'owner="<addr>"'
```

JSON state output includes the masterchain block id, shard block id, proof byte
lengths, and raw state bytes as hex and base64. JSON get-method output includes
the execution block ids, exit code, proof byte lengths, raw result BoC, and a
decoded stack when the current stack decoder supports the returned shape.
`run-abi-get-method` loads ABI JSON, encodes `--arg name=json` inputs through
the ABI metadata, and renders named decoded outputs. It accepts JSON integer
numbers or decimal/hex integer strings, booleans, strings, hex bytes, address
strings, tuple objects, arrays supported by the stack codec, and cell/slice BoC
hex strings. ABI maps and dictionaries are intentionally rejected.

## Wallet Commands

Wallet commands do not store mnemonics or private keys. `wallet generate` is the
only command that prints a mnemonic. Other wallet commands read it from
`--mnemonic-file <path>`, `--mnemonic-file -` for stdin, or
`--mnemonic-env <NAME>`. The default wallet version is V5R1; pass
`--version v4r2` for Wallet V4R2.

```bash
tonutils wallet generate
tonutils wallet address --mnemonic-file seed.txt
tonutils wallet address --version v4r2 --mnemonic-env TON_MNEMONIC
tonutils wallet seqno '<wallet-address>'
tonutils --output hex wallet prepare-transfer --mnemonic-file - --to '<addr>' --amount 100000000 --seqno 0
tonutils wallet send --mnemonic-env TON_MNEMONIC --to '<addr>' --amount 100000000 --deploy
```

Transfer options are `--to <address>`, `--amount <nanotons>`,
`--comment <text>`, `--mode <u8>` defaulting to `3`, `--timeout <seconds>`
defaulting to `60`, optional `--seqno <u32>`, optional `--wallet-id <u32>`,
`--workchain <i8>` defaulting to `0`, and `--deploy` to include `StateInit`.
`prepare-transfer` is offline and requires `--seqno`; `send` fetches `seqno`
unless it is supplied. For `wallet send --deploy`, a missing seqno stack is
treated as seqno `0`; other seqno decoding errors remain errors.

`wallet send` submits one serialized external-in message BoC through
`liteServer.sendMessage` and prints the opaque `SendMsgStatus.status` returned
by the liteserver. That status confirms LiteAPI submission only; it does not
prove transaction inclusion or final execution.

## Advanced Balancer Commands

```bash
tonutils --output json balancer status --num-servers 3
tonutils --rps 5 --global-rps 12 --output json balancer masterchain-info --num-servers 3
```

Balancer commands construct multiple LiteClient peers from the selected config.
They inherit the prototype balancer limits described in
[LiteBalancer](balancer.md).

`--rps` applies to every peer. `--global-rps` applies only to balancer commands
and counts retries and multi-peer message-send attempts as separate requests.

## Offline TVM Commands

BoC decode and TL-B inspection commands do not connect to liteservers and do
not require a global config:

```bash
tonutils --output json tvm boc decode --hex '<boc-hex>'
tonutils --output pretty-json tvm boc decode --base64 '<boc-base64>' --tlb account
tonutils --output json tvm boc decode --file state.boc --tlb block
tonutils --output json tvm boc decode --stdin --tlb proof --verify-proof
tonutils --output json tvm schema check
```

BoC input can be supplied with `--hex`, `--base64`, `--file`, or `--stdin`.
Known TL-B decode values are `message`, `message-relaxed`, `transaction`,
`account`, `block`, `config`, `shard-state`, `proof`, and `merkle-update`.
Proof verification flags only check the synthetic primitive invariant that an
exotic Merkle proof/update child hash equals the hash stored in the exotic
cell. They do not establish liteserver trust or validate a block against a
trusted masterchain root.

## Exit Behavior

Successful commands exit with code `0`. Command line parsing errors, network
errors, LiteAPI errors, and invalid input return nonzero exit codes through the
standard Rust error path. Structured command data is written to stdout; human
diagnostics and connection warnings should be treated as stderr.
