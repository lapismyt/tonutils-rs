//! ABI data model for TON contract descriptions.
//!
//! This module contains the public Rust model, local structural validation for
//! ABI definitions, and scalar ABI value conversion to and from TVM stack
//! entries. It does not parse JSON, build message bodies, or execute contract
//! calls.

include!("abi_parts/part1.rs");
include!("abi_parts/part2.rs");
