//! Low-level smart-contract providers, state reads, and get-method helpers.
//!
//! This crate sits between [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient)
//! and protocol-specific crates such as `tonutils-wallet`, `tonutils-jetton`,
//! and `tonutils-nft`. It does not broadcast messages by itself; callers
//! construct and submit messages through an appropriate provider.
//!
//! Contract reads need a live provider, while address and message construction
//! can be performed offline. Returned data is only as trustworthy as the
//! provider and proof-validation policy selected by the application.

mod provider;
mod stack;

/// LiteAPI `runSmcMethod` mode bit that requests the result stack BoC.
pub const RUN_METHOD_MODE_RETURN_RESULT: u32 = 1 << 2;
