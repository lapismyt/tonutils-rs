//! TL-B model layer.
//!
//! This module provides runtime traits for hand-written TL-B codecs, built-in
//! Initial compatibility blockchain models, and a deterministic schema parser/check-summary
//! workflow in [`schema`]. It intentionally does not include a proc-macro derive
//! crate in Initial compatibility; schema-driven checks and hand-written codecs share the
//! same [`TlbSerialize`] and [`TlbDeserialize`] traits.

pub mod block;
pub mod generated;
pub mod message;
pub mod schema;
pub mod transaction;

mod bits;
mod core;
#[cfg(test)]
mod fixtures;
mod refs;
#[cfg(test)]
mod tests;
mod varuint;

pub use block::{
    BlkMasterInfo, Block, BlockExtra, BlockIdExtTlb, BlockInfo, BlockPrevInfo, ConfigParams,
    ExtBlkRef, GlobalVersion, HashUpdate, McBlockExtra, MerkleProof, MerkleUpdate, ShardIdent,
    ShardState, ShardStateUnsplit, ShardStateUnsplitData, ValueFlow, ValueFlowFees, ValueFlowMain,
};
pub(crate) use core::tlb_to_tvm_error;
pub use core::{
    CellRef, Either, LoadBits, RawCell, StoreBits, TlbDeserialize, TlbError, TlbHashmapE,
    TlbResult, TlbScheme, TlbSerialize, VarUInteger,
};
pub use message::{
    AccStatusChange, Anycast, CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Grams,
    LibRef, Message, MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList,
    SimpleLib, StateInit, StateInitWithLibs, StorageUsed, TickTock, TrActionPhase,
};
pub use tonutils_macros::{Tlb, Tlb as TlbDerive};
pub use transaction::{
    Account, AccountBlock, AccountState, AccountStatus, AccountStorage, ComputeSkipReason,
    DepthBalanceInfo, HashUpdateAccount, ShardAccount, ShardAccountBlocks, ShardAccounts,
    SplitMergeInfo, StorageExtraInfo, StorageInfo, TrBouncePhase, TrComputePhase, TrCreditPhase,
    TrStoragePhase, Transaction, TransactionDescr,
};

use crate::tvm::{Builder, Slice};
use num_bigint::BigUint;

/// Stores a TL-B constructor tag.
pub fn store_tag(builder: &mut Builder, bits: &'static str) -> TlbResult<()> {
    builder.store_tag(bits)
}

/// Checks and consumes a TL-B constructor tag.
pub fn expect_tag(
    slice: &mut Slice,
    constructor: &'static str,
    bits: &'static str,
) -> TlbResult<()> {
    slice.expect_tag(constructor, bits)
}

/// Requires a TL-B slice to be fully consumed.
pub fn ensure_empty(slice: &Slice) -> TlbResult<()> {
    slice.ensure_empty()
}

/// Stores a referenced TL-B value.
pub fn store_ref_tlb<T: TlbSerialize>(builder: &mut Builder, value: &T) -> TlbResult<()> {
    builder.store_ref_tlb(value)
}

/// Loads a referenced TL-B value and requires exact child-cell consumption.
pub fn load_ref_tlb<T: TlbDeserialize>(slice: &mut Slice, schema: &'static str) -> TlbResult<T> {
    slice.load_ref_tlb(schema)
}

/// Stores an optional TL-B value.
pub fn store_maybe<T: TlbSerialize>(builder: &mut Builder, value: &Option<T>) -> TlbResult<()> {
    builder.store_maybe(value)
}

/// Loads an optional TL-B value.
pub fn load_maybe<T: TlbDeserialize>(slice: &mut Slice) -> TlbResult<Option<T>> {
    slice.load_maybe()
}

/// Stores a canonical VarUInteger.
pub fn store_var_uint(builder: &mut Builder, value: &BigUint, length_bits: usize) -> TlbResult<()> {
    builder.store_tlb_var_uint(value, length_bits)
}

/// Loads a canonical VarUInteger.
pub fn load_var_uint(slice: &mut Slice, length_bits: usize) -> TlbResult<BigUint> {
    slice.load_tlb_var_uint(length_bits)
}
