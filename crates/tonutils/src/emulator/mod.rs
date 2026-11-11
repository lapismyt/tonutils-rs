//! Opt-in wrappers around the TON `libemulator` C API.
//!
//! This module mirrors the C++ TVM and transaction emulator used by `pytvm`
//! and exposes the complete set of upstream transactional setters. It is
//! enabled by the `emulator` feature and loads `libemulator` dynamically at
//! runtime, so building this crate never invokes a C++ toolchain.
//!
//! # Native library
//!
//! `libemulator` must be available at runtime. Resolve it in one of these
//! ways:
//!
//! 1. pass an explicit path to [`EmulatorEngine::open`];
//! 2. set `TONUTILS_EMULATOR_LIB`;
//! 3. set `TON_EMULATOR_LIB`;
//! 4. place `libemulator.so`, `libemulator.dylib`, or `libemulator.dll` in the
//!    dynamic loader search path.
//!
//! The library is part of the reference TON build. Build it from
//! `ton-blockchain/ton` with the `emulator` target, or obtain a matching
//! prebuilt artifact from the TON release pipeline.
//!
//! # Transaction emulation
//!
//! ```no_run
//! use tonutils::emulator::{
//!     BlockchainConfig, EmulatorEngine, TransactionEmulator, VmLogVerbosity,
//! };
//!
//! # fn demo(
//! #     config: BlockchainConfig,
//! #     shard_account: tonutils::tlb::ShardAccount,
//! #     message: tonutils::tlb::Message,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let engine = EmulatorEngine::open("/usr/local/lib/libemulator.so")?;
//! let mut emulator = TransactionEmulator::builder(config)
//!     .engine(engine)
//!     .vm_log_verbosity(VmLogVerbosity::Unlimited)
//!     .unixtime(1_700_000_000)
//!     .lt(42_000_000)
//!     .build()?;
//!
//! let result = emulator.emulate_transaction(&shard_account, &message)?;
//! println!("exit actions: {:?}", result.actions);
//! # Ok(())
//! # }
//! ```
//!
//! # Get-methods
//!
//! ```no_run
//! use std::sync::Arc;
//! use tonutils::emulator::{EmulatorEngine, TvmEmulator};
//! use tonutils::tvm::{Cell, VmStack};
//!
//! # fn demo(code: Arc<Cell>, data: Arc<Cell>) -> Result<(), Box<dyn std::error::Error>> {
//! let engine = EmulatorEngine::open_default()?;
//! let mut emulator = TvmEmulator::builder(code, data)
//!     .engine(engine)
//!     .gas_limit(1_000_000)
//!     .build()?;
//!
//! let result = emulator.run_get_method_by_name("seqno", &VmStack::default())?;
//! println!("gas used: {}", result.gas_used);
//! # Ok(())
//! # }
//! ```
//!
//! # Configuration
//!
//! [`BlockchainConfig`] wraps the raw `Hashmap 32 ^Cell` dictionary root that
//! `libemulator` expects. It is not the outer `ConfigParams` cell. Convert a
//! decoded config wrapper with [`BlockchainConfig::from_config_params`]:
//!
//! ```text
//! let decoded = tonutils::liteclient::boc::decode_config_params_boc(&boc)?;
//! let config = BlockchainConfig::from_config_params(&decoded.config);
//! ```
//!
//! The crate intentionally does not embed a default mainnet configuration.
//! Callers choose the config snapshot they trust, which keeps emulation
//! deterministic and lets applications update the snapshot independently.
//!
//! # Stack format
//!
//! Emulator inputs and outputs use the standard [`VmStack`] codec. The
//! higher-level [`TvmStack`](crate::tvm::TvmStack) preserves provider payloads
//! with a crate-specific tag format and is not accepted by `libemulator`.
//! Convert between them when both representations are needed.
//!
//! # Thread safety and blocking
//!
//! [`EmulatorEngine`] is shareable through [`std::sync::Arc`], but each
//! [`TransactionEmulator`] and [`TvmEmulator`] owns a non-thread-safe upstream
//! object. Create one emulator per task. Emulation is synchronous CPU work; in
//! async applications wrap calls in `tokio::task::spawn_blocking`.
//!
//! # Trust boundary
//!
//! Emulation runs locally, but its result is only as trustworthy as the
//! caller-supplied config, shard account, message, and library binary. The
//! module performs no network requests and does not fetch configuration.

use std::ffi::CString;

pub mod config;
pub mod engine;
pub mod error;
mod ffi;
pub mod transaction;
pub mod tvm;
pub mod types;

#[cfg(test)]
mod tests;

pub use crate::tvm::{VmSlice, VmStack, VmStackValue};
pub use config::BlockchainConfig;
pub use engine::{EMULATOR_LIB_ENV, EMULATOR_LIB_FALLBACK_ENV, EmulatorEngine, EmulatorLogLevel};
pub use error::EmulatorError;
pub use transaction::{TransactionEmulator, TransactionEmulatorBuilder};
pub use tvm::{C7Config, TvmEmulator, TvmEmulatorBuilder};
pub use types::{
    GetMethodResult, MessageEmulationResult, TransactionEmulationResult, VmLogVerbosity,
};

pub(crate) fn c_string(value: &str, field: &str) -> Result<CString, EmulatorError> {
    CString::new(value)
        .map_err(|_| EmulatorError::InvalidInput(format!("{field} contains an interior NUL byte")))
}
