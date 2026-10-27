//! Offline wallet data, signing, and external-message construction.
//!
//! Wallet address derivation and transfer BoC construction are offline. Enable
//! the `provider` feature only when using helpers that fetch state such as a
//! wallet sequence number:
//!
//! ```toml
//! tonutils-wallet = { version = "2", features = ["provider"] }
//! ```
//!
//! The crate never stores credentials for the caller. Treat mnemonic material
//! as secret, review wallet version and workchain choices, and remember that a
//! successful LiteAPI submission is not proof of transaction inclusion.
