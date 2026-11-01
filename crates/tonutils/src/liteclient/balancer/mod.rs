//! Lite client balancer for load balancing and failover across multiple liteservers.
//!
//! This module provides a `LiteBalancer` that manages multiple `LiteClient` instances,
//! automatically handling:
//! - Connection failures and failover
//! - Load balancing based on response times and current load
//! - Peer health checking and automatic reconnection
//! - Best-effort synchronization filtering based on observed masterchain seqnos
//! - Archival node detection

mod archive;
mod execute;
#[macro_use]
pub mod macros;
#[cfg(test)]
mod reliability_tests;
mod selection;
mod state;
#[cfg(test)]
mod tests;
mod types;

pub use types::{BalancerError, LiteBalancer};

pub type BalancerResult<T> = Result<T, BalancerError>;
