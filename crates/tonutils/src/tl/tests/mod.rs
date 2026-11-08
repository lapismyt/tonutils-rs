//! Tests for TL (Type Language) module

#[cfg(feature = "liteclient")]
mod common;
#[cfg(feature = "liteclient")]
mod golden;
mod schema_audit;
mod schema_check;

#[cfg(feature = "liteclient")]
use common::*;
