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

The account TL-B slice in `src/tlb/transaction.rs` covers:

- `StorageExtraInfo`: `storage_extra_none$000` and
  `storage_extra_info$001 dict_hash:uint256`;
- `StorageInfo`: `used:StorageUsed`, `storage_extra:StorageExtraInfo`,
  `last_paid:uint32`, and `due_payment:(Maybe Grams)`;
- `AccountState`: `account_uninit$00`, `account_frozen$01 state_hash:bits256`,
  and `account_active$1 _:StateInit`;
- `AccountStorage`: `last_trans_lt:uint64`, `balance:CurrencyCollection`, and
  `state:AccountState`;
- `Account`: `account_none$0` or
  `account$1 addr:MsgAddressInt storage_stat:StorageInfo storage:AccountStorage`;
- `ShardAccount`: `account:^Account`, `last_trans_hash:uint256`, and
  `last_trans_lt:uint64`.

`DepthBalanceInfo` is implemented as
`depth_balance$_ split_depth:(#<= 30) balance:CurrencyCollection`, with
`split_depth` encoded in five bits and constrained to `0..=30`.
`ShardAccounts` wraps `HashmapAugE 256 ShardAccount DepthBalanceInfo`; its
empty form still carries the top-level `DepthBalanceInfo` augmentation.

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

The current TL-B model slice includes schema-exact action phase metadata:
`tr_phase_action$_` stores booleans for `success`, `valid`, and `no_funds`,
`AccStatusChange`, optional forward/action fees, signed result code and optional
argument, four `uint16` counters, `action_list_hash:bits256`, and
`tot_msg_size:StorageUsed`. The action phase links to the produced action list
only by `action_list_hash`; it does not embed `OutList`.

Transaction-description TL-B models are implemented in `src/tlb/transaction.rs`.
They cover storage, credit, compute, bounce, split/merge info, and the complete
`TransactionDescr` constructor family:

- ordinary (`trans_ord$0000`);
- storage-only (`trans_storage$0001`);
- tick-tock (`trans_tick_tock$001`);
- split-prepare (`trans_split_prepare$0100`);
- split-install (`trans_split_install$0101`);
- merge-prepare (`trans_merge_prepare$0110`);
- merge-install (`trans_merge_install$0111`).

`action:(Maybe ^TrActionPhase)` is represented as `Option<TrActionPhase>` and
encoded through an exact child reference when present. The recursive
`prepare_transaction:^Transaction` field in split/merge install descriptions is
represented as `Box<Transaction>` and decoded from an exact child reference.

Top-level `transaction$0111` is implemented with exact child-reference layout:
the parent stores account hash, logical times, `now:uint32`, `outmsg_cnt:uint15`,
original/final `AccountStatus`, total fees, and references to
`HASH_UPDATE Account` and `TransactionDescr`. The message child reference stores
`in_msg:(Maybe ^(Message Any))` followed by
`out_msgs:(HashmapE 15 ^(Message Any))`. Outbound dictionary keys must be
15 bits, and referenced message, state-update, and description cells are decoded
exactly.

`update_hashes#72 old_hash:bits256 new_hash:bits256 = HASH_UPDATE Account` is
represented as the concrete `HashUpdateAccount` type. A generic public
`HashUpdate<T>` is intentionally deferred until another schema slice needs it.

Transaction history pagination usually uses `(account, lt, hash)`.

## Account Blocks

Per-shard transaction collections use augmented dictionaries so validators can
commit aggregate balance data at internal dictionary nodes without scanning all
leaves:

- `AccountBlock` maps upstream
  `acc_trans#5 account_addr:bits256 transactions:(HashmapAug 64 ^Transaction CurrencyCollection) state_update:^(HASH_UPDATE Account)`.
  The transaction dictionary is a non-empty `HashmapAug` keyed by 64-bit
  logical time. Leaf values are referenced `Transaction` cells and leaf/fork
  augmentations are `CurrencyCollection`.
- `ShardAccountBlocks` wraps
  `HashmapAugE 256 AccountBlock CurrencyCollection`, keyed by 256-bit account
  address hash. The top-level `HashmapAugE` extra is preserved even when the
  dictionary is empty.

The generic dictionary layer preserves decoded leaf, fork, and top-level
augmentation values. It does not infer TON-specific aggregation rules; callers
that construct augmented dictionaries must supply the augmentation values.

## Messages

Message families:

- internal message,
- external inbound message,
- external outbound message.

Messages contain `CommonMsgInfo`, optional state init, and body.

## State

Shard state contains account collections and metadata. Masterchain state contains global config and references to shard states. Proof verification requires decoding enough state structure to verify inclusion paths.

## Crate Mapping

Current crate has low-level cell/address support plus focused TL-B models for
messages, outbound action lists, transaction action phase metadata, and
transaction descriptions, account state, `ShardAccount`, `HASH_UPDATE Account`,
top-level `Transaction`, augmented `ShardAccounts`, `AccountBlock`, and
`ShardAccountBlocks`. Full block headers, value flow, `BlockExtra`, shard-state
models, and config params should be introduced in future TL-B slices.

## Missing Work

- TL-B definitions for full blocks, value flow, shard hashes, and shard state.
- Proof path extraction from shard state.
- Golden BoC fixtures for real account and transaction cells.
