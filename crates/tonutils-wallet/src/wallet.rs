//! Offline wallet helpers.
//!
//! The first wallet surface covers offline Wallet V4R2 and V5R1 helpers.
//! It intentionally starts with deterministic cell construction, address
//! derivation, signing, and external message BoC assembly; live send helpers
//! are thin provider adapters.

pub(super) use ed25519_dalek::{Signer, SigningKey};
pub(super) use num_bigint::{BigInt, BigUint, Sign};
pub(super) use std::sync::Arc;
pub(super) use tonutils_tlb::{
    CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Either, Grams, Message,
    MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList, StateInit,
    TlbDeserialize, TlbError, TlbSerialize, ensure_empty,
};
#[cfg(any())]
pub(super) use tonutils_tvm::BitKey;
pub(super) use tonutils_tvm::{Address, Builder, Cell, HashmapE, Slice, serialize_boc};

mod code;
mod errors;
mod message;
mod mnemonic;
mod provider;
#[cfg(any())]
mod tests;
mod v4r2;
mod v5r1;

use mnemonic::*;
use v5r1::*;

pub use message::*;
pub use mnemonic::*;
