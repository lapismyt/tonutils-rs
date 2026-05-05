# Examples

Examples are compiled with explicit feature requirements in `Cargo.toml`. They
are live-network examples at runtime, but they exit successfully when the
required environment variables are missing so CI can compile them without
network credentials.

## Compile Examples

```bash
cargo check --examples --features network-config
```

## Available Examples

- `liteclient_masterchain_info` requires `liteclient` and `network-config`.
  It reads `TON_GLOBAL_CONFIG_JSON`, connects to liteserver index `0`, and
  prints the latest masterchain seqno.
- `liteclient_raw_query` requires `liteclient` and `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON` and `TON_LITEAPI_REQUEST_HEX`, sends already
  serialized LiteAPI bytes through `query_raw`, and prints the raw response as
  hex.
- `network_config` requires `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON`, parses the liteserver list, and prints indexed
  socket addresses.
- `contract_get_state` requires `liteclient` and `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON` and `TON_CONTRACT_ADDRESS`, fetches latest account
  state, and prints block ids plus raw state length.
- `contract_get_method` requires `liteclient` and `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON`, `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, runs an empty-stack get-method, and prints the exit code
  plus raw result length.
- `litebalancer_failover` requires `liteclient` and `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON`, connects to all available liteservers from config,
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
- `tvm_stack_run_method` requires `liteclient` and `network-config`. It reads
  `TON_GLOBAL_CONFIG_JSON`, `TON_CONTRACT_ADDRESS`, and optional
  `TON_GET_METHOD`, creates a non-empty `TvmStack`, calls
  `run_get_method_by_name`, and prints exit code plus result size.

Remaining coverage gaps tracked in `TODO.md`: proof and mempool examples.
