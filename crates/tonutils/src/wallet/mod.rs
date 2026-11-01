//! Offline wallet data, signing, and external-message construction.
//!
//! Wallet address derivation, signing, and transfer BoC construction are
//! offline. Applications that need live state, such as a wallet sequence
//! number, should fetch it through [`crate::liteclient`] or another provider
//! and pass it into these helpers:
//!
//! ```toml
//! tonutils = { version = "2", features = ["wallet", "liteclient"] }
//! ```
//!
//! The crate never stores credentials for the caller. Treat mnemonic material
//! as secret, review wallet version and workchain choices, and remember that a
//! successful LiteAPI submission is not proof of transaction inclusion.

#[path = "wallet.rs"]
mod wallet_impl;

pub use wallet_impl::{
    MAINNET_GLOBAL_ID, TESTNET_GLOBAL_ID, TonMnemonic, WALLET_V4R2_CODE_HASH,
    WALLET_V4R2_DEFAULT_ID, WALLET_V5R1_CODE_HASH, WALLET_V5R1_MAINNET_DEFAULT_ID,
    WALLET_V5R1_TESTNET_DEFAULT_ID, WalletError, WalletMessage, WalletV4R2, WalletV4R2Data,
    WalletV4R2SignedBody, WalletV5R1, WalletV5R1Context, WalletV5R1Data, WalletV5R1ExtendedAction,
    WalletV5R1ExtendedActionList, WalletV5R1Extensions, WalletV5R1ExternalBody,
    WalletV5R1SignedBody, WalletV5R1WalletId, WalletVersion, ton_mnemonic_seed, wallet_v4r2_code,
    wallet_v5r1_code,
};
