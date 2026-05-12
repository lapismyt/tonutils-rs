//! Offline wallet helpers.
//!
//! The first wallet surface covers offline Wallet V4R2 and V5R1 helpers.
//! It intentionally starts with deterministic cell construction, address
//! derivation, signing, and external message BoC assembly; live send helpers
//! are thin provider adapters.

include!("wallet_parts/part1.rs");
include!("wallet_parts/part2.rs");
include!("wallet_parts/part3.rs");
