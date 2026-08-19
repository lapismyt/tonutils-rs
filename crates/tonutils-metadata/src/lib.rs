//! TEP-64 metadata parsing and lossless content helpers.
//!
//! The parser handles off-chain snake content and on-chain dictionaries while
//! preserving the original content cell for unsupported or partially known
//! fields. It is fully offline and has no network or credential requirements.
//! Pair it with [`tonutils_jetton`](https://docs.rs/tonutils-jetton) or
//! [`tonutils_nft`](https://docs.rs/tonutils-nft) for standard get-method data.
//!
//! All parsing is offline and preserves unsupported content where possible;
//! this crate does not fetch metadata, contact liteservers, or verify asset
//! authenticity.

#[path = "metadata.rs"]
pub mod metadata;

pub use metadata::*;
