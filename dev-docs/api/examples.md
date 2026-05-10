# API Examples To Maintain

This page defines examples that should exist in README, doctests, or `examples/` once APIs stabilize.

## LiteClient Connect

Expected flow:

1. Resolve `TON_NETWORK`, defaulting to mainnet.
2. Load `TON_GLOBAL_CONFIG_JSON` or download the public network config.
3. Select `TON_LS_INDEX`, defaulting to `0`.
4. Connect with ADNL TCP.
5. Fetch masterchain info.

## Network Config

Expected flow:

1. Resolve `TON_NETWORK`, defaulting to mainnet.
2. Load `TON_GLOBAL_CONFIG_JSON` or download the public network config.
3. Parse liteserver entries.
4. Print indexed socket addresses for follow-up examples.

## Raw LiteAPI

Expected flow:

1. Load config through the live-network defaults.
2. Select one liteserver with `TON_LS_INDEX`.
3. Use `TON_LITEAPI_REQUEST_HEX` when provided.
4. Otherwise serialize `liteServer.getTime`.
5. Send bytes with `query_raw` and print raw response bytes as hex.

## LiteBalancer

Expected flow:

1. Build peer descriptors from config loaded through the live-network defaults.
2. Connect or lazy-connect peers.
3. Fetch version/time/masterchain info.
4. Close background tasks.

## Get-Method

Expected flow:

1. Load config through the live-network defaults.
2. Parse `TON_CONTRACT_ADDRESS` or use the documented mainnet default.
3. When `TON_NETWORK=testnet`, require `TON_CONTRACT_ADDRESS` for default
   `seqno` get-method examples until a stable testnet contract is documented.
4. Fetch latest block context.
5. Build TVM stack.
6. Run method by name.
7. Check exit code.
8. Decode typed stack result.

## Send Message

Expected flow:

1. Build external message cell.
2. Serialize BoC.
3. Send via LiteAPI.
4. Track message hash until transaction appears.

## Mempool Stream

Future flow:

1. Build scanner from overlay/DHT config.
2. Subscribe to pending messages.
3. Filter by account or shard.
4. Deduplicate by message hash.
5. Emit pending and finalized stages.
