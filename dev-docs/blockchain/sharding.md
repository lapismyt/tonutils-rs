# Sharding

TON shards split account space so transactions can be processed in parallel.

## Shard Identifier

Shard ids are 64-bit prefixes. The high bit is significant. The full shard is represented by:

```text
0x8000000000000000
```

Do not format shard ids only as signed decimals in diagnostics; preserve hex output for clarity.

## Account To Shard

An account belongs to a shard based on the prefix of its 256-bit account id. As shards split and merge, the shard covering an account can change over time.

## Masterchain Relation

Masterchain blocks reference shardchain blocks. A light client verifies shard data by proving the shard block is referenced by a verified masterchain block.

## LiteAPI Methods

Relevant methods:

```tl
liteServer.getShardInfo ...
liteServer.getAllShardsInfo ...
liteServer.getShardBlockProof ...
```

## SDK Requirements

- Determine account shard at a given masterchain block.
- Fetch shard block proof.
- Verify shard inclusion.
- Handle shard split and merge history.

## Missing Work

- Shard descriptor TLB decoder.
- Account-to-shard helper.
- Split/merge traversal helpers.
