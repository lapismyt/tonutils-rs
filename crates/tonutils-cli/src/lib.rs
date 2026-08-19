//! Command-line parsing and rendering for TON SDK workflows.
//!
//! The companion `tonutils` binary is intended for shell scripts and
//! diagnostics. Network commands require a live liteserver configuration;
//! TVM, BoC, and schema commands are deterministic and offline.
//!
//! Prefer the public binary for end-user automation and use this crate when a
//! Rust application needs to embed the [`Cli`] parser or output model.
//!
//! The binary and parser cover LiteAPI, contract, TVM, BoC, wallet, and schema
//! commands. Network subcommands need live configuration; TVM and schema
//! operations remain deterministic and offline.

#[path = "mod.rs"]
pub mod cli;

pub use cli::*;
