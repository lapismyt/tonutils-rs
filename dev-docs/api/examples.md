# API Examples To Maintain

This page defines examples that should exist in README, doctests, or `examples/` once APIs stabilize.

## LiteClient Connect

Expected flow:

1. Parse global config.
2. Select liteserver.
3. Connect with ADNL TCP.
4. Fetch masterchain info.

## LiteBalancer

Expected flow:

1. Build peer descriptors from config.
2. Connect or lazy-connect peers.
3. Fetch version/time/masterchain info.
4. Close background tasks.

## Get-Method

Expected flow:

1. Parse contract address.
2. Fetch latest block context.
3. Build TVM stack.
4. Run method by name.
5. Check exit code.
6. Decode typed stack result.

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
