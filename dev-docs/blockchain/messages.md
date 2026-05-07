# Messages And Transactions

Messages trigger account transactions and are the main object users send to contracts.

This page tracks the message and transaction-description subset currently mapped
in `src/tlb/message.rs` and `src/tlb/transaction.rs`.
The source of truth is upstream `ton-blockchain/ton`
`crypto/block/block.tlb` at
`https://github.com/ton-blockchain/ton/blob/master/crypto/block/block.tlb`,
especially the `MsgAddressInt`, `MsgAddressExt`, `MsgAddress`, `Grams`,
`CurrencyCollection`, `StateInit`, `StateInitWithLibs`, `CommonMsgInfo`,
`CommonMsgInfoRelaxed`, `Message`, `MessageRelaxed`, `LibRef`, `OutAction`,
`TrStoragePhase`, `TrCreditPhase`, `TrComputePhase`, `TrBouncePhase`,
`SplitMergeInfo`, and `TransactionDescr` definitions.
The overview page at
`https://docs.ton.org/v3/documentation/data-formats/layout/messages` is useful
for conceptual orientation, but constructor tags and field order come from the
upstream TL-B schema.

## Message Families

TON messages are represented by TLB constructors. The major families are:

- internal messages between contracts,
- external inbound messages from outside the blockchain,
- external outbound messages emitted outside the blockchain.

The implemented `CommonMsgInfo` constructors are:

- `int_msg_info$0`: internal message metadata.
- `ext_in_msg_info$10`: inbound external message metadata.
- `ext_out_msg_info$11`: outbound external message metadata.

The implemented `CommonMsgInfoRelaxed` constructors are:

- `int_msg_info$0`: internal relaxed metadata.
- `ext_out_msg_info$11`: outbound external relaxed metadata.

Relaxed common info intentionally has no `ext_in_msg_info$10` constructor.
Its internal constructor uses `src:MsgAddress`, so the source can be internal
or external, while `dest` remains `MsgAddressInt`.

## Common Message Info

Common message info stores routing and fee metadata. Fields vary by message family but can include:

- source address,
- destination address,
- value,
- import fee,
- ihr fee,
- forward fee,
- creation logical time,
- creation unix time,
- bounce flags.

For current upstream internal messages, this crate stores
`extra_flags:(VarUInteger 16)` between `value:CurrencyCollection` and
`fwd_fee:Grams`. Older schema notes or SDKs may call this position `ihr_fee`;
that is not the field implemented here.

## State Init

A message can carry `StateInit` to deploy a contract. State init can include:

- fixed prefix length,
- special tick/tock flags,
- code cell,
- data cell,
- libraries.

The implemented upstream form is:

- `fixed_prefix_length:(Maybe (## 5))`;
- `special:(Maybe TickTock)`;
- `code:(Maybe ^Cell)`;
- `data:(Maybe ^Cell)`;
- `library:(Maybe ^Cell)`.

`StateInitWithLibs` is also implemented for message validation paths. It uses
the same fixed prefix, special, code, and data fields as `StateInit`, but stores
`library:(HashmapE 256 SimpleLib)` instead of `Maybe ^Cell`.

`SimpleLib` is:

- `public:Bool`;
- `root:^Cell`.

The 256-bit `StateInitWithLibs.library` key is the library hash. Values preserve
the referenced library root cell exactly.

## Addresses

Internal message addresses support:

- `addr_std$10 anycast:(Maybe Anycast) workchain_id:int8 address:bits256`;
- `addr_var$11 anycast:(Maybe Anycast) addr_len:(## 9) workchain_id:int32 address:(bits addr_len)`.

`addr_std` maps to the existing `Address` type for the workchain and 256-bit
hash while preserving optional anycast. `addr_var` stores raw address bits and
the exact bit length.

External message addresses support:

- `addr_none$00`;
- `addr_extern$01 len:(## 9) external_address:(bits len)`.

The external model stores raw bits in `Vec<u8>` plus a bit length. It does not
use the older `ExternalAddress` helper because that helper is limited to a
`u64` value and cannot represent arbitrary 511-bit external addresses.

`MsgAddress` wraps either `MsgAddressInt` or `MsgAddressExt` by preserving the
underlying address constructor. The first two bits therefore still select
`00`, `01`, `10`, or `11`; there is no additional wrapper tag.

## Values

`Grams` wraps `nanograms$_ amount:(VarUInteger 16) = Grams`. Encodings must be
canonical: zero uses a zero byte-length prefix, and non-zero values use the
shortest big-endian byte representation.

`CurrencyCollection` stores `grams:Grams` and an extra-currency dictionary:
`HashmapE 32 (VarUInteger 32)`. Dictionary keys are fixed-width 32-bit currency
ids. Values are canonical `VarUInteger 32` payloads.

## Body

The implemented `Message` type models `Message Any`:

- `info: CommonMsgInfo`;
- `init:(Maybe (Either StateInit ^StateInit))`;
- `body:(Either Cell ^Cell)`, represented as `Either<Arc<Cell>, Arc<Cell>>`.

Inline and referenced placement is preserved explicitly. Inline body decoding
consumes all remaining bits and references into a new cell. Referenced body
decoding loads exactly one child reference; `Message::from_cell` then rejects
trailing parent bits or references.

The implemented `MessageRelaxed` type models `MessageRelaxed Any`:

- `info: CommonMsgInfoRelaxed`;
- `init:(Maybe (Either StateInit ^StateInit))`;
- `body:(Either Cell ^Cell)`, represented as `Either<Arc<Cell>, Arc<Cell>>`.

It uses the same inline versus referenced state-init and body placement rules as
`Message Any`. This is the form used by `action_send_msg` in transaction action
lists.

The body is usually a cell. For wallet messages it often contains:

- operation code,
- query id,
- transfer parameters,
- comments or payload references.

## Out Actions

`OutAction` models the closed action family emitted by the TVM action phase:

- `action_send_msg#0ec3c86d mode:(## 8) out_msg:^(MessageRelaxed Any)`;
- `action_set_code#ad4de08e new_code:^Cell`;
- `action_reserve_currency#36e6b809 mode:(## 8) currency:CurrencyCollection`;
- `action_change_library#26fa1dd4 mode:(## 7) libref:LibRef`.

`action_send_msg` always stores the relaxed outbound message as an exact child
cell. `action_change_library` stores a seven-bit mode, so Rust serialization
rejects values above 127.

`LibRef` has two constructors:

- `libref_hash$0 lib_hash:bits256`, mapped to `[u8; 32]`;
- `libref_ref$1 library:^Cell`, preserving the referenced library cell.

`OutList` stores actions as the upstream recursive linked list:

- `out_list_empty$_ = OutList 0`, encoded as an empty cell with no bits and no
  references;
- `out_list$_ {n:#} prev:^(OutList n) action:OutAction = OutList (n + 1)`,
  encoded as one previous-list reference followed by the current `OutAction`.

The Rust model exposes the list as `Vec<OutAction>` in schema/execution order.
The first vector item is stored deepest next to `out_list_empty$_`; the last
item is stored in the root node. TON limits action lists to 255 actions. The
codec rejects serialization and decoding above that limit and decodes each
previous-list reference exactly.

`TrActionPhase` stores the action phase result metadata, but it does not embed
`OutList`. The upstream field is `action_list_hash:bits256`, which is the hash
of the resulting action list. Decode `OutList` separately when a c5/action-list
cell is available, then compare or store its cell hash through
`TrActionPhase.action_list_hash`.

The implemented action phase fields are:

- `success:Bool`, `valid:Bool`, and `no_funds:Bool`;
- `status_change:AccStatusChange`, with tags `0`, `10`, and `11`;
- `total_fwd_fees:(Maybe Grams)` and `total_action_fees:(Maybe Grams)`;
- `result_code:int32` and `result_arg:(Maybe int32)`;
- `tot_actions:uint16`, `spec_actions:uint16`, `skipped_actions:uint16`, and
  `msgs_created:uint16`;
- `action_list_hash:bits256`;
- `tot_msg_size:StorageUsed`, where `StorageUsed` is `cells:(VarUInteger 7)`
  and `bits:(VarUInteger 7)`.

## Transaction Phases

`src/tlb/transaction.rs` implements the transaction phases that are needed by
`TransactionDescr`:

- `tr_phase_storage$_ storage_fees_collected:Grams storage_fees_due:(Maybe Grams) status_change:AccStatusChange`;
- `tr_phase_credit$_ due_fees_collected:(Maybe Grams) credit:CurrencyCollection`;
- `tr_phase_compute_skipped$0 reason:ComputeSkipReason`;
- `tr_phase_compute_vm$1 success:Bool msg_state_used:Bool account_activated:Bool gas_fees:Grams ^[...]`;
- `tr_phase_bounce_negfunds$00`;
- `tr_phase_bounce_nofunds$01 msg_size:StorageUsed req_fwd_fees:Grams`;
- `tr_phase_bounce_ok$1 msg_size:StorageUsed msg_fees:Grams fwd_fees:Grams`.

`ComputeSkipReason` uses the upstream tags `00` (`cskip_no_state`), `01`
(`cskip_bad_state`), `10` (`cskip_no_gas`), and `110`
(`cskip_suspended`). The `111` bit pattern is invalid and is rejected.

The VM compute phase stores `success`, `msg_state_used`, `account_activated`,
and `gas_fees` in the parent cell, then stores the remaining VM metadata in an
exact child reference:

- `gas_used:(VarUInteger 7)` and `gas_limit:(VarUInteger 7)`, with at most six
  payload bytes;
- `gas_credit:(Maybe (VarUInteger 3))`, with at most two payload bytes;
- `mode:int8`, `exit_code:int32`, and `exit_arg:(Maybe int32)`;
- `vm_steps:uint32`;
- `vm_init_state_hash:bits256` and `vm_final_state_hash:bits256`.

The child reference is decoded exactly. Trailing bits or references in the VM
tail are reported as an invalid `TrComputePhase.vm` reference payload.

## Transaction Descriptions

The implemented `TransactionDescr` constructors are:

- `trans_ord$0000 credit_first:Bool storage_ph:(Maybe TrStoragePhase) credit_ph:(Maybe TrCreditPhase) compute_ph:TrComputePhase action:(Maybe ^TrActionPhase) aborted:Bool bounce:(Maybe TrBouncePhase) destroyed:Bool`;
- `trans_storage$0001 storage_ph:TrStoragePhase`;
- `trans_tick_tock$001 is_tock:Bool storage_ph:TrStoragePhase compute_ph:TrComputePhase action:(Maybe ^TrActionPhase) aborted:Bool destroyed:Bool`;
- `trans_split_prepare$0100 split_info:SplitMergeInfo storage_ph:(Maybe TrStoragePhase) compute_ph:TrComputePhase action:(Maybe ^TrActionPhase) aborted:Bool destroyed:Bool`;
- `trans_split_install$0101 split_info:SplitMergeInfo prepare_transaction:^Transaction installed:Bool`;
- `trans_merge_prepare$0110 split_info:SplitMergeInfo storage_ph:TrStoragePhase aborted:Bool`;
- `trans_merge_install$0111 split_info:SplitMergeInfo prepare_transaction:^Transaction storage_ph:(Maybe TrStoragePhase) credit_ph:(Maybe TrCreditPhase) compute_ph:TrComputePhase action:(Maybe ^TrActionPhase) aborted:Bool destroyed:Bool`.

`action:(Maybe ^TrActionPhase)` maps to `Option<TrActionPhase>`, but it is
encoded through a referenced child cell exactly as the schema requires. A
present action stores a `1` bit and a child reference containing
`TrActionPhase`; an absent action stores only `0`. Referenced action payloads
must consume all child bits and references.

`SplitMergeInfo` is an implicit constructor containing
`cur_shard_pfx_len:(## 6)`, `acc_split_depth:(## 6)`, `this_addr:bits256`, and
`sibling_addr:bits256`. Serialization rejects prefix lengths above 63.

The split-install and merge-install constructors currently preserve
`prepare_transaction:^Transaction` as typed `Box<Transaction>` values decoded
from exact child references. The box is only Rust layout indirection for the
recursive schema; it is not an extra TL-B layer.

## Transaction Relation

Each transaction has at most one inbound message and zero or more outbound messages. To confirm a sent external message, locate the transaction whose inbound message hash matches the sent message hash.

## SDK Requirements

- Build external inbound messages from the hand-written TL-B model.
- Build internal messages from the hand-written TL-B model.
- Compute message hash from the resulting cell.
- Decode inbound and outbound messages from transactions.
- Track sent message inclusion.

## Missing Work

- Golden BoC fixtures for real upstream or liteserver messages.
- Typed message body wrappers for wallet, jetton, and contract-specific bodies.
- Wallet-specific message builders.
- Transaction location helpers.
- Fee estimation helpers.
