# TON Blockchain Data Model

TON is a sharded blockchain. A correct SDK must model masterchain, workchains, shardchains, blocks, accounts, transactions, messages, and state proofs.

## Chains

Important chain ids:

- `-1`: masterchain.
- `0`: basechain.
- other signed values: additional workchains.

The masterchain stores global consensus data, validator-related data, config, and references to shardchain blocks.

## Shards

A shard is identified by a signed 64-bit shard prefix. The full shard is commonly represented by:

```text
0x8000000000000000
```

When stored as `i64`, this value is negative. APIs must preserve the exact bits and not reinterpret it as a decimal semantic value.

## Block Ids

LiteAPI uses:

```tl
tonNode.blockId workchain:int shard:long seqno:int = tonNode.BlockId;
tonNode.blockIdExt workchain:int shard:long seqno:int root_hash:int256 file_hash:int256 = tonNode.BlockIdExt;
```

`BlockIdExt` is required for fetching and verifying concrete blocks because it contains both hashes.

## Accounts

An account is identified by:

- workchain id,
- 256-bit account id.

Account state includes:

- account status,
- balance,
- last transaction logical time and hash,
- code cell,
- data cell,
- storage statistics.

## Transactions

Transactions are ordered per account by logical time. A transaction can include:

- inbound message,
- outbound messages,
- total fees,
- state update,
- compute phase,
- action phase,
- bounce phase,
- storage phase,
- credit phase.

Transaction history pagination usually uses `(account, lt, hash)`.

## Messages

Message families:

- internal message,
- external inbound message,
- external outbound message.

Messages contain `CommonMsgInfo`, optional state init, and body.

## State

Shard state contains account collections and metadata. Masterchain state contains global config and references to shard states. Proof verification requires decoding enough state structure to verify inclusion paths.

## Crate Mapping

Current crate has only low-level cell and address support. Full blockchain data models should be introduced under a future TLB module.

## Missing Work

- TLB definitions for blocks, accounts, transactions, messages.
- Account state decoder.
- Transaction decoder.
- Message decoder.
- Proof path extraction from shard state.
