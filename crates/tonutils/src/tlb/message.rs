//! Hand-written TL-B codecs for core blockchain message models.

#[cfg(test)]
pub(super) use crate::tlb::{Either, TlbDeserialize, TlbError, TlbSerialize, store_tag};
#[cfg(test)]
pub(super) use num_bigint::BigUint;
#[cfg(test)]
pub(super) use std::sync::Arc;
#[cfg(test)]
pub(super) use tonutils::tvm::{Address, HashmapE};
#[cfg(test)]
pub(super) use tonutils::tvm::{Builder, Cell, Slice};

mod actions;
mod address;
mod currency;
mod helpers;
mod info;
#[expect(
    clippy::module_inception,
    reason = "The message owner keeps its wire model in a sibling module of the same name"
)]
mod message;
mod phases;
mod state_init;
#[cfg(test)]
mod tests;

pub use address::{
    Anycast, CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Grams, Message,
    MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, SimpleLib, StateInit,
    StateInitWithLibs, TickTock,
};
pub use helpers::{AccStatusChange, LibRef, OutAction, OutList, StorageUsed, TrActionPhase};

#[cfg(test)]
use address::VAR_UINT_16_LEN_BITS;
use address::{
    ACTION_CHANGE_LIBRARY_TAG, ACTION_RESERVE_CURRENCY_TAG, ACTION_SEND_MSG_TAG,
    ACTION_SET_CODE_TAG, MAX_OUT_LIST_ACTIONS, VAR_UINT_7_LEN_BITS, VAR_UINT_7_MAX_BYTES,
};
use helpers::{
    load_maybe_anycast, load_maybe_ref_cell, load_maybe_tick_tock, load_message_body,
    load_message_init, load_state_init_prefix, load_two_bit_tag, store_maybe_anycast,
    store_maybe_tick_tock, store_message_body, store_message_init, store_state_init_prefix,
    validate_bit_string,
};
