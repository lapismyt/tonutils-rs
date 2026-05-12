//! Lite client balancer for load balancing and failover across multiple liteservers.
//!
//! This module provides a `LiteBalancer` that manages multiple `LiteClient` instances,
//! automatically handling:
//! - Connection failures and failover
//! - Load balancing based on response times and current load
//! - Peer health checking and automatic reconnection
//! - Best-effort synchronization filtering based on observed masterchain seqnos
//! - Archival node detection

include!("balancer_parts/part1.rs");
include!("balancer_parts/part2.rs");
include!("balancer_parts/part3.rs");
include!("balancer_parts/part4.rs");
