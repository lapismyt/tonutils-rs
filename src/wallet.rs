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
pub(super) use crate::tvm::{Address, BitKey, Builder, Cell, HashmapE, Slice, serialize_boc};
pub(super) use bip39::{Language, Mnemonic};
pub(super) use ed25519_dalek::{Signer, SigningKey};
pub(super) use hmac::{Hmac, Mac};
pub(super) use num_bigint::{BigInt, BigUint, Sign};
pub(super) use pbkdf2::pbkdf2_hmac;
pub(super) use rand::RngCore;
pub(super) use sha2::Sha512;
pub(super) use std::sync::atomic::{AtomicBool, Ordering};
pub(super) use std::sync::mpsc;
pub(super) use std::sync::{Arc, OnceLock};
pub(super) use std::thread;

mod code;
mod errors;
mod message;
mod mnemonic;
mod provider;
#[cfg(test)]
mod tests;
mod v4r2;
mod v5r1;

use code::*;
use errors::*;
use message::*;
use mnemonic::*;
use provider::*;
#[cfg(test)]
use tests::*;
use v4r2::*;
use v5r1::*;

pub use code::*;
pub use errors::*;
pub use message::*;
pub use mnemonic::*;
pub use provider::*;
pub use v4r2::*;
pub use v5r1::*;
