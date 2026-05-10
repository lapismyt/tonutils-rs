# LiteAPI TL Schema

LiteAPI is defined in `lite_api.tl` in the upstream TON repository. Local schema copies live under `src/tl/schemas/`.

## Schema Revision

- Synced local `src/tl/schemas/lite_api.tl` on 2026-05-05 from:
  - `https://github.com/ton-blockchain/ton/blob/master/tl/generate/scheme/lite_api.tl`
- Added upstream nonfinal pending-shard constructors:
  - `liteServer.nonfinal.pendingShardBlocks`
  - `liteServer.nonfinal.getPendingShardBlocks`
- Synced `liteServer.nonfinal.getValidatorGroups` flag layout to upstream:
  - `shard:mode.0?long` (previous local snapshot used `mode.1`)

## Common Types

```tl
liteServer.accountId workchain:int id:int256 = liteServer.AccountId;
liteServer.transactionId3 account:int256 lt:long = liteServer.TransactionId3;
liteServer.libraryEntry hash:int256 data:bytes = liteServer.LibraryEntry;
```

`AccountId` is the LiteAPI form of an internal account address. It stores the workchain id and 256-bit account hash.

## Masterchain Types

```tl
liteServer.masterchainInfo last:tonNode.blockIdExt state_root_hash:int256 init:tonNode.zeroStateIdExt = liteServer.MasterchainInfo;
liteServer.masterchainInfoExt mode:# version:int capabilities:long last:tonNode.blockIdExt last_utime:int now:int state_root_hash:int256 init:tonNode.zeroStateIdExt = liteServer.MasterchainInfoExt;
```

Use `last` as the reference for current chain state. `state_root_hash` identifies the current state root. `init` identifies zerostate.

## Block Data And State

```tl
liteServer.blockData id:tonNode.blockIdExt data:bytes = liteServer.BlockData;
liteServer.blockState id:tonNode.blockIdExt root_hash:int256 file_hash:int256 data:bytes = liteServer.BlockState;
liteServer.blockHeader id:tonNode.blockIdExt mode:# header_proof:bytes = liteServer.BlockHeader;
```

Returned `data` and proof fields are usually BoC or serialized proof data. They are not automatically verified.

## Account State

```tl
liteServer.accountState id:tonNode.blockIdExt shardblk:tonNode.blockIdExt shard_proof:bytes proof:bytes state:bytes = liteServer.AccountState;
```

Fields:

- `id`: requested block id.
- `shardblk`: shard block containing the account state.
- `shard_proof`: proof linking shard to masterchain context.
- `proof`: account proof.
- `state`: account state data.

`state` is a BoC whose root is the TL-B `Account` value returned by the
liteserver. It is not a standalone `ShardAccount`; the `ShardAccount` appears
inside the account-proof path and must be extracted from the verified
`ShardAccounts` dictionary before its `last_trans_hash` can be trusted.
For full accounts, `Account.storage.last_trans_lt` is available directly from
the `state` cell.

The `shard_proof` and `proof` byte fields are BoCs used for proof material.
Account-state proof payloads can contain more than one BoC root; official proof
flow substitutes the returned `state` cell into the pruned proof tree before
checking hashes. Current typed helpers decode these roots for diagnostics but do
not claim proof validity.

## Run Method

```tl
liteServer.runSmcMethod mode:# id:tonNode.blockIdExt account:liteServer.accountId method_id:long params:bytes = liteServer.RunMethodResult;
```

Result:

```tl
liteServer.runMethodResult mode:# id:tonNode.blockIdExt shardblk:tonNode.blockIdExt shard_proof:mode.0?bytes proof:mode.0?bytes state_proof:mode.1?bytes init_c7:mode.3?bytes lib_extras:mode.4?bytes exit_code:int result:mode.2?bytes = liteServer.RunMethodResult;
```

The `exit_code` is contract execution output, not a transport error.

## Transaction Listing

```tl
liteServer.listBlockTransactions id:tonNode.blockIdExt mode:# count:# after:mode.7?liteServer.transactionId3 reverse_order:mode.6?true want_proof:mode.5?true = liteServer.BlockTransactions;
liteServer.listBlockTransactionsExt id:tonNode.blockIdExt mode:# count:# after:mode.7?liteServer.transactionId3 reverse_order:mode.6?true want_proof:mode.5?true = liteServer.BlockTransactionsExt;
```

Use `after` for pagination. `incomplete` in the response indicates that more data is available.

## Proofs

```tl
liteServer.getBlockProof mode:# known_block:tonNode.blockIdExt target_block:mode.0?tonNode.blockIdExt = liteServer.PartialBlockProof;
liteServer.getShardBlockProof id:tonNode.blockIdExt = liteServer.ShardBlockProof;
```

Block proofs link known and target blocks. Shard block proofs link shard blocks to a masterchain block.

## Queues And Pending Data

```tl
liteServer.getOutMsgQueueSizes mode:# wc:mode.0?int shard:mode.0?long = liteServer.OutMsgQueueSizes;
liteServer.getDispatchQueueInfo mode:# id:tonNode.blockIdExt after_addr:mode.1?int256 max_accounts:int want_proof:mode.0?true = liteServer.DispatchQueueInfo;
liteServer.getDispatchQueueMessages mode:# id:tonNode.blockIdExt addr:int256 after_lt:long max_messages:int want_proof:mode.0?true one_account:mode.1?true messages_boc:mode.2?true = liteServer.DispatchQueueMessages;
```

These are important for future pending-message and mempool-adjacent features.

## Nonfinal Data

Current local schema snapshot (`src/tl/schemas/lite_api.tl`) includes nonfinal validator-group, candidate, and pending-shard APIs:

```tl
liteServer.nonfinal.getValidatorGroups mode:# wc:mode.0?int shard:mode.0?long = liteServer.nonfinal.ValidatorGroups;
liteServer.nonfinal.getCandidate id:liteServer.nonfinal.candidateId = liteServer.nonfinal.Candidate;
liteServer.nonfinal.getPendingShardBlocks mode:# wc:mode.0?int shard:mode.0?long = liteServer.nonfinal.PendingShardBlocks;
```

## Implementation Checklist

- Keep schema copy synchronized with upstream.
- Add typed request and response structs.
- Add response conversion helpers.
- Add client method.
- Add TL roundtrip test.
- Add constructor id test.
- Add live test only if response can be stable enough.
