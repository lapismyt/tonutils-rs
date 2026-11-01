//! Hand-written TL-B codecs for account state, transactions, descriptions, and phases.

#[cfg(test)]
pub(super) use crate::tlb::{AccStatusChange, CurrencyCollection, MsgAddressInt, StateInit};
#[cfg(test)]
pub(super) use crate::tlb::{Grams, Message, StorageUsed, TrActionPhase};
#[cfg(test)]
pub(super) use crate::tlb::{
    TlbDeserialize, TlbError, TlbSerialize, load_maybe, store_ref_tlb, store_tag, tlb_to_tvm_error,
};
#[cfg(test)]
pub(super) use num_bigint::BigUint;
#[cfg(test)]
pub(super) use tonutils::tvm::HashmapE;
#[cfg(test)]
pub(super) use tonutils::tvm::{Builder, Slice};

mod account;
mod description;
mod helpers;
mod phases;
mod shard;
#[cfg(test)]
mod tests;
#[expect(
    clippy::module_inception,
    reason = "The transaction owner keeps its wire model in a sibling module of the same name"
)]
mod transaction;

pub use account::{
    Account, AccountBlock, AccountState, AccountStatus, AccountStorage, ComputeSkipReason,
    DepthBalanceInfo, HashUpdateAccount, ShardAccount, ShardAccountBlocks, ShardAccounts,
    StorageExtraInfo, StorageInfo, TrComputePhase, TrCreditPhase, TrStoragePhase, Transaction,
};
pub use helpers::{SplitMergeInfo, TrBouncePhase, TransactionDescr};

#[cfg(test)]
use account::OUT_MSG_KEY_BITS;
use account::{
    VAR_UINT_3_LEN_BITS, VAR_UINT_3_MAX_BYTES, VAR_UINT_7_LEN_BITS, VAR_UINT_7_MAX_BYTES,
};
use helpers::{
    expect_tag_bits, expect_u8_tag, load_maybe_ref_message, load_tag_bit, load_three_bit_tag,
    load_two_bit_tag, store_maybe_i32, store_maybe_ref_message, store_maybe_var_uint_3,
    store_var_uint_7,
};
