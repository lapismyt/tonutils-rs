# Examples

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
- `TON_CONTRACT_ADDRESS`: account address for contract examples. Mainnet
  contract examples default to
  `UQBg0E2FCj7kkYWw-2yEcOHs7p1xtnqAoLIYBUG2AJ56eFNP`.
- `TON_GET_METHOD`: get-method name, defaulting to `seqno`.
- `TON_LITEAPI_REQUEST_HEX`: serialized LiteAPI request bytes for raw queries,
  defaulting to serialized `liteServer.getTime`.

Example commands:

```bash
cargo run -F full --example network_config
cargo run -F full --example liteclient_masterchain_info
TON_NETWORK=testnet cargo run -F full --example liteclient_masterchain_info
TON_LS_INDEX=2 cargo run -F full --example liteclient_raw_query
TON_CONTRACT_ADDRESS=EQ... cargo run -F full --example contract_get_method
```

## Available Examples

- `liteclient_masterchain_info` requires `liteclient`, `network-config`, and `cli`.
  It loads config from the live-network defaults, connects to `TON_LS_INDEX`,
  and prints the latest masterchain seqno.
- `liteclient_raw_query` requires `liteclient`, `network-config`, and `cli`. It reads
  live-network defaults and optional `TON_LITEAPI_REQUEST_HEX`, sends already
  serialized LiteAPI bytes through `query_raw`, and prints the raw response as
  hex. Without `TON_LITEAPI_REQUEST_HEX`, it sends `liteServer.getTime`.
- `network_config` requires `network-config` and `cli`. It reads
  live-network defaults, parses the liteserver list, and prints indexed socket
  addresses.
- `contract_get_state` requires `liteclient`, `network-config`, and `cli`. It reads
  live-network defaults and optional `TON_CONTRACT_ADDRESS`, fetches latest
  account state, and prints block ids plus raw state length.
- `contract_get_method` requires `liteclient`, `network-config`, and `cli`. It reads
  live-network defaults, optional `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, runs an empty-stack get-method, and prints the exit code
  plus raw result length. With `TON_NETWORK=testnet`, set
  `TON_CONTRACT_ADDRESS`; otherwise the example exits successfully because no
  stable default testnet `seqno` contract is defined.
- `litebalancer_failover` requires `liteclient`, `network-config`, and `cli`. It reads
  live-network defaults, connects to all available liteservers from config,
  initializes `LiteBalancer`, performs `get_masterchain_info`, and prints
  seqno plus alive and archival peer counts.
- `adnl_ping` requires `adnl-tcp`. It performs a loopback-safe ADNL handshake
  roundtrip in-memory (`to_bytes` + `decrypt_from_raw`) and prints sender and
  receiver identifiers.
- `tvm_cell_builder` requires `tvm`. It builds a cell with fixed-width integer,
  big unsigned integer, and big signed integer values, reads them back via
  `Slice`, and prints decoded values.
- `tvm_boc_roundtrip` requires `tvm`. It builds a small referenced cell graph,
  serializes it into BoC with CRC, deserializes back, and prints basic
  structure metadata.
- `tvm_stack_run_method` requires `liteclient`, `network-config`, and `cli`. It reads
  live-network defaults, optional `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, calls `run_get_method_by_name` with an empty `TvmStack`,
  and prints exit code plus result size. With `TON_NETWORK=testnet`, set
  `TON_CONTRACT_ADDRESS`.
- `tlb_schema_codegen` requires `tvm`. It parses the local Phase 1
  upstream-derived TL-B schema slice, regenerates the checked summary, and
  prints whether the checked-in output matches.
- `tvm_boc_decode` requires `tvm`. It builds an offline `Account::None`
  fixture, encodes it as BoC, decodes it, and prints the root hash plus typed
  account view.
- `liteclient_account_state_decode` requires `liteclient`. It decodes
  `TON_ACCOUNT_STATE_BOC_HEX` when set and otherwise exits cleanly using an
  offline `Account::None` fixture.
- `proof_verify` requires `tvm`. It reads `TON_MERKLE_PROOF_BOC_HEX` when set
  and checks the exotic Merkle proof child-hash invariant. Without the
  environment variable, it uses a deterministic offline Merkle proof fixture.
- `tvm_dictionary_roundtrip` requires `tvm`. It builds an offline
  compatibility `Dict` backed by `HashmapE`, serializes and deserializes it,
  and prints the key size, entry count, and root hash.
- `tlb_message_roundtrip` requires `tvm`. It builds deterministic internal
  messages with inline and referenced bodies, roundtrips them through TL-B, and
  prints body placement, value, and root hash.
- `tlb_transaction_roundtrip` requires `tvm`. It builds a minimal ordinary
  transaction with empty inbound and outbound messages, roundtrips it through
  TL-B, and prints logical time, statuses, and fee.
- `tlb_account_state_roundtrip` requires `tvm`. It builds a full account with
  empty `StateInit`, storage, and balance fields, roundtrips it through TL-B,
  and prints the address hash, state, and balance.
- `tlb_block_wrapper_decode` requires `tvm`. It builds the Phase 1 raw-cell
  `Block` wrapper with deterministic child cells, decodes it, and prints the
  global id plus child hashes.
- `tlb_config_params_wrapper` requires `tvm`. It builds `ConfigParams` around a
  deterministic raw config dictionary root, roundtrips it through TL-B, and
  prints the config address and dictionary root hash.
- `tlb_parse_boc` requires `tvm`. It decodes `TON_TLB_BOC_HEX` as a typed
  `Account` root, or uses an offline `Account::None` fixture, then prints
  typed data and hash roundtrip information.
- `tlb_read_tx_data` requires `tvm`. It accepts `TON_TRANSACTION_BOC_HEX` or
  `TON_TRANSACTION_BOC_BASE64`, decodes a `Transaction`, and prints logical
  time, account hash, fees, statuses, message summary, and root hash. Without
  input it uses an offline deterministic transaction fixture.
- `tlb_custom_derive` requires `tlb-derive`. It demonstrates a custom
  TEP-74-style jetton transfer struct with a hex constructor tag, inferred
  unsigned field width, wrapper TL-B fields, and generated roundtrip codecs.
- `wallet_offline_transfer` requires `tvm`. It derives V4R2 and V5R1
  addresses from a fixed TON mnemonic and builds a signed V4R2 deployment
  transfer BoC without network access.

Remaining coverage gaps tracked in `TODO.md`: live proof capture and mempool
examples.
