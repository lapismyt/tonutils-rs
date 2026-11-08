//! Tests for TL (Type Language) module

#[cfg(feature = "liteclient")]
mod common;
#[cfg(feature = "adnl")]
mod cross_sdk;
#[cfg(feature = "liteclient")]
mod golden;
#[cfg(feature = "adnl")]
#[path = "live_wire.rs"]
mod live_wire;
mod schema_audit;
mod schema_check;

#[cfg(feature = "liteclient")]
use common::*;
