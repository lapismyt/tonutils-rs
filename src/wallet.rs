//! Offline wallet helpers.
//!
//! The first wallet surface covers offline Wallet V4R2 and V5R1 helpers.
//! It intentionally starts with deterministic cell construction, address
//! derivation, signing, and external message BoC assembly; live send helpers
//! are thin provider adapters.

pub(super) use crate::tlb::{
    CommonMsgInfo, CommonMsgInfoRelaxed, CurrencyCollection, Either, Grams, Message,
    MessageRelaxed, MsgAddress, MsgAddressExt, MsgAddressInt, OutAction, OutList, StateInit,
    TlbDeserialize, TlbError, TlbSerialize, ensure_empty,
};
#[cfg(test)]
pub(super) use crate::tvm::BitKey;
pub(super) use crate::tvm::{Address, Builder, Cell, HashmapE, Slice, serialize_boc};
pub(super) use ed25519_dalek::{Signer, SigningKey};
pub(super) use num_bigint::{BigInt, BigUint, Sign};
pub(super) use std::sync::Arc;

mod code;
mod errors;
mod message;
mod mnemonic;
mod provider;
#[cfg(test)]
mod tests;
mod v4r2;
mod v5r1;

use mnemonic::*;
use v5r1::*;

pub use message::*;
pub use mnemonic::*;
