# tonutils-rs Examples: Offline and Live TON Workflows

Examples are compiled with explicit feature requirements in `Cargo.toml`. They
include offline TVM/TL-B examples and live-network examples. Live-network
examples default to public mainnet configuration so they can be run directly.
Environment variables remain overrides for local configs, testnet, selected
liteservers, contract addresses, methods, and raw requests.

Audience: users looking for copyable crate workflows and contributors checking
public API examples. Prerequisites vary by example; each example below lists
the required feature group. Offline examples do not need network access.

## Compile Examples

```bash
cargo check --examples --all-features
```

## Live-Network Defaults

By default, live examples download `https://ton.org/global.config.json`, select
liteserver index `0`, and use mainnet. Set `TON_NETWORK=testnet` to download
`https://ton.org/testnet-global.config.json` instead. Set
`TON_GLOBAL_CONFIG_JSON` to bypass downloading and provide a full config JSON
string directly.

Common variables:

- `TON_NETWORK`: `mainnet` or `testnet`, defaulting to `mainnet`.
- `TON_GLOBAL_CONFIG_JSON`: full TON global config JSON. Overrides public
  config download.
- `TON_LS_INDEX`: liteserver index for single-peer examples, defaulting to `0`.
- `TON_CONTRACT_ADDRESS`: account address for contract examples. Contract
  examples require an explicit address and exit successfully when it is absent.
- `TON_GET_METHOD`: get-method name, defaulting to `seqno`.
- `TON_LITEAPI_REQUEST_HEX`: serialized LiteAPI request bytes for raw queries,
  defaulting to serialized `liteServer.getTime`.

Example commands:

```bash
cargo run -p tonutils-cli -- --network "${TON_NETWORK:-mainnet}" status
cargo run -p tonutils-cli -- --network "${TON_NETWORK:-mainnet}" \
  liteclient masterchain-info --ls-index "${TON_LS_INDEX:-0}"
cargo run -p tonutils-cli -- --network testnet \
  liteclient masterchain-info --ls-index "${TON_LS_INDEX:-0}"
cargo run -p tonutils-cli -- --network "${TON_NETWORK:-mainnet}" \
  liteclient time --ls-index "${TON_LS_INDEX:-2}"
cargo run -p tonutils-cli -- --network "${TON_NETWORK:-mainnet}" \
  contract run-get-method --ls-index "${TON_LS_INDEX:-0}" \
  --address "${TON_CONTRACT_ADDRESS:?set TON_CONTRACT_ADDRESS}" \
  --method "${TON_GET_METHOD:-seqno}"
```

The retired `full` feature no longer exists. Validate every workspace crate
with all optional functionality enabled using:

```bash
cargo check --workspace --all-features
```

## Available Examples

Examples are organized by feature area under `examples/`:

### ADNL (`examples/adnl/`)

- `adnl_ping` requires `adnl-tcp`. It performs a loopback-safe ADNL handshake
  roundtrip in-memory (`to_bytes` + `decrypt_from_raw`) and prints sender and
  receiver identifiers.

```bash
cargo run -p tonutils --example adnl_ping --features adnl-tcp
```

### Network (`examples/network/`)

- `network_config` requires `liteclient-network-config`, `cli`. It reads
  live-network defaults, parses the liteserver list, and prints indexed socket
  addresses.

```bash
cargo run -p tonutils --example network_config --features liteclient-network-config,cli
```

### LiteClient (`examples/liteclient/`)

- `liteclient_masterchain_info` requires `liteclient-network-config`, `cli`.
  It loads config from the live-network defaults, connects to `TON_LS_INDEX`,
  and prints the latest masterchain seqno.

```bash
cargo run -p tonutils --example liteclient_masterchain_info --features liteclient-network-config,cli
```

- `liteclient_raw_query` requires `liteclient-network-config`, `cli`. It reads
  live-network defaults and optional `TON_LITEAPI_REQUEST_HEX`, sends already
  serialized LiteAPI bytes through `query_raw`, and prints the raw response as
  hex. Without `TON_LITEAPI_REQUEST_HEX`, it sends `liteServer.getTime`.

```bash
cargo run -p tonutils --example liteclient_raw_query --features liteclient-network-config,cli
```

- `litebalancer_failover` requires `liteclient-network-config`, `cli`. It reads
  live-network defaults, connects to all available liteservers from config,
  initializes `LiteBalancer`, performs `get_masterchain_info`, and prints
  seqno plus alive and archival peer counts.

```bash
cargo run -p tonutils --example litebalancer_failover --features liteclient-network-config,cli
```

- `liteclient_account_state_decode` requires `liteclient-network-config`, `tvm`. It decodes
  `TON_ACCOUNT_STATE_BOC_HEX` when set and otherwise exits cleanly using an
  offline `Account::None` fixture.

```bash
cargo run -p tonutils --example liteclient_account_state_decode --features liteclient-network-config,tvm
```

### Contracts (`examples/contracts/`)

- `contract_get_state` requires `liteclient-network-config`, `cli`. It reads
  live-network defaults and optional `TON_CONTRACT_ADDRESS`, fetches latest
  account state, and prints block ids plus raw state length.

```bash
cargo run -p tonutils --example contract_get_state --features liteclient-network-config,cli
```

- `contract_get_method` requires `liteclient-network-config`, `cli`. It reads
  live-network defaults, `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, runs an empty-stack get-method, and prints the exit code
  plus raw result length. Set `TON_CONTRACT_ADDRESS`; otherwise the example
  exits successfully without making a get-method request.

```bash
cargo run -p tonutils --example contract_get_method --features liteclient-network-config,cli
```

- `tvm_stack_run_method` requires `liteclient-network-config`, `cli`, `tvm`. It reads
  live-network defaults, `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, calls `run_get_method_by_name` with an empty `TvmStack`,
  and prints exit code plus result size. Set `TON_CONTRACT_ADDRESS`; otherwise
  the example exits successfully without making a get-method request.

```bash
cargo run -p tonutils --example tvm_stack_run_method --features liteclient-network-config,cli,tvm
```

### TVM (`examples/tvm/`)

- `proof_verify` requires `tvm`. It reads `TON_MERKLE_PROOF_BOC_HEX` when set
  and checks the exotic Merkle proof child-hash invariant. Without the
  environment variable, it uses a deterministic offline Merkle proof fixture.

```bash
cargo run -p tonutils --example proof_verify --features tvm
```

- `tvm_boc_decode` requires `tvm`. It builds an offline `Account::None`
  fixture, encodes it as BoC, decodes it, and prints the root hash plus typed
  account view.

```bash
cargo run -p tonutils --example tvm_boc_decode --features tvm
```

- `tvm_boc_roundtrip` requires `tvm`. It builds a small referenced cell graph,
  serializes it into BoC with CRC, deserializes back, and prints basic
  structure metadata.

```bash
cargo run -p tonutils --example tvm_boc_roundtrip --features tvm
```

- `tvm_cell_builder` requires `tvm`. It builds a cell with fixed-width integer,
  big unsigned integer, and big signed integer values, reads them back via
  `Slice`, and prints decoded values.

```bash
cargo run -p tonutils --example tvm_cell_builder --features tvm
```

- `tvm_dictionary_roundtrip` requires `tvm`. It builds an offline
  compatibility `Dict` backed by `HashmapE`, serializes and deserializes it,
  and prints the key size, entry count, and root hash.

```bash
cargo run -p tonutils --example tvm_dictionary_roundtrip --features tvm
```

### TLB (`examples/tlb/`)

- `tlb_schema_codegen` requires `tvm`. It parses the local Initial compatibility
  upstream-derived TL-B schema snapshot, regenerates the checked summary, and
  prints whether the checked-in output matches.

```bash
cargo run -p tonutils --example tlb_schema_codegen --features tvm
```

- `tlb_account_state_roundtrip` requires `tvm`. It builds a full account with
  empty `StateInit`, storage, and balance fields, roundtrips it through TL-B,
  and prints the address hash, state, and balance.

```bash
cargo run -p tonutils --example tlb_account_state_roundtrip --features tvm
```

- `tlb_block_wrapper_decode` requires `tvm`. It builds the Initial compatibility raw-cell
  `Block` wrapper with deterministic child cells, decodes it, and prints the
  global id plus child hashes.

```bash
cargo run -p tonutils --example tlb_block_wrapper_decode --features tvm
```

- `tlb_config_params_wrapper` requires `tvm`. It builds `ConfigParams` around a
  deterministic raw config dictionary root, roundtrips it through TL-B, and
  prints the config address and dictionary root hash.

```bash
cargo run -p tonutils --example tlb_config_params_wrapper --features tvm
```

- `tlb_custom_derive` requires `tvm`, `tlb-derive`. It demonstrates a custom
  TEP-74-style jetton transfer struct with a hex constructor tag, inferred
  unsigned field width, wrapper TL-B fields, and generated roundtrip codecs.

```bash
cargo run -p tonutils --example tlb_custom_derive --features tvm,tlb-derive
```

- `tlb_message_roundtrip` requires `tvm`. It builds deterministic internal
  messages with inline and referenced bodies, roundtrips them through TL-B, and
  prints body placement, value, and root hash.

```bash
cargo run -p tonutils --example tlb_message_roundtrip --features tvm
```

- `tlb_transaction_roundtrip` requires `tvm`. It builds a minimal ordinary
  transaction with empty inbound and outbound messages, roundtrips it through TL-B,
  and prints logical time, statuses, and fee.

```bash
cargo run -p tonutils --example tlb_transaction_roundtrip --features tvm
```

- `tlb_parse_boc` requires `tvm`. It decodes `TON_TLB_BOC_HEX` as a typed
  `Account` root, or uses an offline `Account::None` fixture, then prints
  typed data and hash roundtrip information.

```bash
cargo run -p tonutils --example tlb_parse_boc --features tvm
```

- `tlb_read_tx_data` requires `tvm`. It accepts `TON_TRANSACTION_BOC_HEX` or
  `TON_TRANSACTION_BOC_BASE64`, decodes a `Transaction`, and prints logical
  time, account hash, fees, statuses, message summary, and root hash. Without
  input it uses an offline deterministic transaction fixture.

```bash
cargo run -p tonutils --example tlb_read_tx_data --features tvm
```

### Wallet (`examples/wallet/`)

- `wallet_offline_transfer` requires `tvm`, `wallet-provider`. It derives V4R2 and V5R1
  addresses from a fixed TON mnemonic and builds a signed V4R2 deployment
  transfer BoC without network access.

```bash
cargo run -p tonutils --example wallet_offline_transfer --features tvm,wallet-provider
```

### Mempool (`examples/mempool/`)

The `tonutils-mempool` crate ships the following examples. They require the
`tonutils-mempool` package (feature `mempool` in the `tonutils` facade):

- `mempool_stream` joins the mainnet mempool overlay over ADNL/UDP
  using the downloaded global config, streams accepted external messages,
  re-broadcasts each one to the connected members, and shuts down cleanly on
  Ctrl-C.

```bash
cargo run -p tonutils-mempool --example mempool_stream
```

- `mempool_scan` (live acceptance counters).

```bash
cargo run -p tonutils-mempool --example mempool_scan
```

- `peer_store_scan` (the same scan with a durable peer store that
  makes the next launch skip discovery).

```bash
cargo run -p tonutils-mempool --example peer_store_scan -- [global-config.json] [peer-store.json]
```

- `mempool_scan_fast` (optimized scanning with pinned peers).

```bash
cargo run -p tonutils-mempool --example mempool_scan_fast -- [global-config.json]
```

- `mempool_send_v5r1` (send external message via V5R1 wallet).

```bash
cargo run -p tonutils-mempool --example mempool_send_v5r1 -- ...
```

- `adnl_udp_mempool` (raw ADNL/UDP overlay session).

```bash
cargo run -p tonutils-mempool --example adnl_udp_mempool
```

- `dht_diag` (DHT diagnostic tool).

```bash
cargo run -p tonutils-mempool --example dht_diag
```

See `docs/mempool.md` for the mempool guide.

Remaining coverage gaps tracked in `TODO.md`: live proof capture.