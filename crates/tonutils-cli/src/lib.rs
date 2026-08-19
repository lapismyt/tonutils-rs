//! Command-line parsing and rendering for TON workflows.
//!
//! The companion `tonutils` binary is intended for shell scripts and
//! diagnostics. Network commands require a live liteserver configuration;
//! TVM, BoC, and schema commands are deterministic and offline.
//!
//! Prefer the public binary for end-user automation and use this crate when a
//! Rust application needs to embed the [`Cli`] parser or output model.

#[path = "mod.rs"]
pub mod cli;

pub use cli::*;
