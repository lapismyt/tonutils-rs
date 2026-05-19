//! TEP-62 NFT metadata helpers.
//!
//! This module decodes the `get_collection_data()`, `get_nft_data()`, and
//! `get_nft_content()` stack layouts used by TEP-62 NFT contracts, then maps
//! full TEP-64 content into NFT-oriented metadata fields. Off-chain JSON
//! fetching, transfers, royalty helpers, and indexer integration are
//! intentionally outside this layer.

mod decode;
mod metadata;
mod payload;
#[cfg(test)]
mod payload_tests;
mod provider;
#[cfg(test)]
mod tests;
mod types;

pub use payload::*;
pub use types::*;
