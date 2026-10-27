//! Offline wallet helpers.
//!
//! The first wallet surface covers offline Wallet V4R2 and V5R1 helpers.
//! It intentionally starts with deterministic cell construction, address
//! derivation, signing, and external message BoC assembly; live send helpers
//! are thin provider adapters.

#[cfg(test)]
pub(super) use std::sync::Arc;
#[cfg(test)]
pub(super) use tonutils::tlb::{
    CommonMsgInfo, Message, MsgAddressInt, TlbDeserialize, TlbSerialize,
};
#[cfg(test)]
pub(super) use tonutils::tvm::{Address, HashmapE, Slice};

mod code;
mod errors;
mod message;
mod mnemonic;
mod provider;
#[cfg(test)]
mod tests;
mod v4r2;
mod v5r1;

pub use message::{
    WalletV4R2, WalletV4R2SignedBody, WalletV5R1, WalletV5R1ExtendedAction,
    WalletV5R1ExtendedActionList, WalletV5R1ExternalBody, WalletV5R1SignedBody,
};
pub use mnemonic::{
    MAINNET_GLOBAL_ID, TESTNET_GLOBAL_ID, TonMnemonic, WALLET_V4R2_CODE_HASH,
    WALLET_V4R2_DEFAULT_ID, WALLET_V5R1_CODE_HASH, WALLET_V5R1_MAINNET_DEFAULT_ID,
    WALLET_V5R1_TESTNET_DEFAULT_ID, WalletError, WalletMessage, WalletV4R2Data, WalletV5R1Context,
    WalletV5R1Data, WalletV5R1Extensions, WalletV5R1WalletId, WalletVersion, ton_mnemonic_seed,
    wallet_v4r2_code, wallet_v5r1_code,
};

#[cfg(test)]
use mnemonic::{
    TON_DEFAULT_SEED, TON_DEFAULT_SEED_ITERATIONS, TON_SEED_VERSION, TON_SEED_VERSION_ITERATIONS,
};
