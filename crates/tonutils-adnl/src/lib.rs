//! Native Rust ADNL protocol primitives and optional TCP transport for TON.
//!
//! Use this crate when an application needs ADNL identities, packet framing,
//! or the optional native TCP transport without depending on the higher-level
//! [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient) client.
//!
//! The default `tcp` feature enables the asynchronous transport. Disable it
//! for protocol types only:
//!
//! ```toml
//! tonutils-adnl = { version = "2", default-features = false }
//! ```
//!
//! ADNL is a transport primitive, not a trust or proof layer. Applications
//! remain responsible for selecting trusted liteservers and validating the
//! responses consumed by higher-level workflows.
//!
//! The protocol types are usable offline; the `tcp` feature adds Tokio-based
//! live transport. See [`tonutils_liteclient`](https://docs.rs/tonutils-liteclient)
//! for the higher-level LiteAPI client.

#[path = "adnl/mod.rs"]
pub mod adnl;

pub use adnl::*;
