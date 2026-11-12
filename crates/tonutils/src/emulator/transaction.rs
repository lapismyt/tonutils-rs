//! Transaction emulation wrapper.

use std::sync::Arc;

use crate::tlb::{Message, ShardAccount, TlbSerialize};
use crate::tvm::{Cell, VmStackValue, boc::boc_to_base64};

use super::c_string;
use super::config::BlockchainConfig;
use super::engine::EmulatorEngine;
use super::error::EmulatorError;
use super::types::{TransactionEmulationResult, VmLogVerbosity, parse_transaction_result};

/// A `libemulator` transaction emulator.
///
/// The value owns an opaque upstream object and is intentionally neither
/// `Send` nor `Sync`; create one per task or thread.
#[derive(Debug)]
pub struct TransactionEmulator {
    engine: Arc<EmulatorEngine>,
    handle: *mut std::ffi::c_void,
}

impl TransactionEmulator {
    /// Creates an emulator with the default library and VM log verbosity.
    ///
    /// # Errors
    ///
    /// Propagates library loading and emulator creation errors.
    pub fn new(config: BlockchainConfig) -> Result<Self, EmulatorError> {
        Self::builder(config).build()
    }

    /// Creates a builder for a fully configured transaction emulator.
    pub fn builder(config: BlockchainConfig) -> TransactionEmulatorBuilder {
        TransactionEmulatorBuilder::new(config)
    }

    /// Sets the emulated Unix timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_unixtime(&mut self, unixtime: u32) -> Result<(), EmulatorError> {
        // SAFETY: `handle` is a live object created by this engine, and the
        // setter accepts a `uint32_t`.
        if unsafe { (self.engine.ffi.transaction_emulator_set_unixtime)(self.handle, unixtime) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_unixtime",
            })
        }
    }

    /// Sets the emulated logical time.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_lt(&mut self, lt: u64) -> Result<(), EmulatorError> {
        // SAFETY: `handle` is live and the setter accepts a `uint64_t`.
        if unsafe { (self.engine.ffi.transaction_emulator_set_lt)(self.handle, lt) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_lt",
            })
        }
    }

    /// Sets the deterministic random seed.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_rand_seed(&mut self, seed: [u8; 32]) -> Result<(), EmulatorError> {
        let encoded = c_string(&hex::encode(seed), "rand_seed")?;
        // SAFETY: `handle` is live and `encoded` is a NUL-terminated 64-byte
        // hex string, matching the upstream contract.
        if unsafe {
            (self.engine.ffi.transaction_emulator_set_rand_seed)(self.handle, encoded.as_ptr())
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_rand_seed",
            })
        }
    }

    /// Controls whether CHKSIG operations succeed unconditionally.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_ignore_chksig(&mut self, ignore: bool) -> Result<(), EmulatorError> {
        // SAFETY: `handle` is live and the setter accepts a C `bool`.
        if unsafe { (self.engine.ffi.transaction_emulator_set_ignore_chksig)(self.handle, ignore) }
        {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_ignore_chksig",
            })
        }
    }

    /// Replaces the blockchain configuration used by this emulator.
    ///
    /// # Errors
    ///
    /// Propagates BoC serialization errors and
    /// [`EmulatorError::Rejected`] when the library rejects the config.
    pub fn set_config(&mut self, config: &BlockchainConfig) -> Result<(), EmulatorError> {
        let encoded = c_string(&config.to_base64()?, "config_boc")?;
        // SAFETY: `handle` is live and `encoded` is a NUL-terminated base64 BoC.
        if unsafe {
            (self.engine.ffi.transaction_emulator_set_config)(self.handle, encoded.as_ptr())
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_config",
            })
        }
    }

    /// Sets the shared-library dictionary.
    ///
    /// Passing `None` is a no-op because the upstream setter requires a BoC.
    ///
    /// # Errors
    ///
    /// Propagates BoC serialization errors and
    /// [`EmulatorError::Rejected`] when the library rejects the dictionary.
    pub fn set_libraries(&mut self, libraries: Option<&Arc<Cell>>) -> Result<(), EmulatorError> {
        let Some(libraries) = libraries else {
            return Ok(());
        };
        let encoded = c_string(&boc_to_base64(libraries, false)?, "libraries_boc")?;
        // SAFETY: `handle` is live and `encoded` is a NUL-terminated base64 BoC
        // of a `HashmapE 256 ^Cell` dictionary.
        if unsafe { (self.engine.ffi.transaction_emulator_set_libs)(self.handle, encoded.as_ptr()) }
        {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_libs",
            })
        }
    }

    /// Enables or disables TVM debug primitives.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_debug_enabled(&mut self, enabled: bool) -> Result<(), EmulatorError> {
        // SAFETY: `handle` is live and the setter accepts a C `bool`.
        if unsafe { (self.engine.ffi.transaction_emulator_set_debug_enabled)(self.handle, enabled) }
        {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_debug_enabled",
            })
        }
    }

    /// Sets the previous-block info tuple used as the 13th `c7` element.
    ///
    /// Pass `None` to leave the option unchanged. Pass a tuple or `null`
    /// [`VmStackValue`] to update it.
    ///
    /// # Errors
    ///
    /// Propagates stack serialization errors and
    /// [`EmulatorError::Rejected`] when the library rejects the tuple.
    pub fn set_prev_blocks_info(
        &mut self,
        info: Option<&VmStackValue>,
    ) -> Result<(), EmulatorError> {
        let encoded = info
            .map(|value| value.to_cell().map_err(EmulatorError::from))
            .transpose()?
            .map(|cell| boc_to_base64(&cell, false))
            .transpose()?
            .map(|text| c_string(&text, "prev_blocks_info_boc"))
            .transpose()?;

        // SAFETY: `handle` is live; the pointer is either null (unchanged) or a
        // NUL-terminated base64 BoC of a single VmStackValue.
        if unsafe {
            (self.engine.ffi.transaction_emulator_set_prev_blocks_info)(
                self.handle,
                encoded
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
            )
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "transaction_emulator_set_prev_blocks_info",
            })
        }
    }

    /// Emulates an internal or external message transaction.
    ///
    /// # Errors
    ///
    /// Propagates serialization errors and returns
    /// [`EmulatorError::Emulation`] with the upstream failure details when the
    /// transaction cannot be emulated.
    pub fn emulate_transaction(
        &mut self,
        shard_account: &ShardAccount,
        message: &Message,
    ) -> Result<TransactionEmulationResult, EmulatorError> {
        parse_transaction_result(&self.emulate_transaction_raw(shard_account, message)?)
    }

    /// Emulates a transaction and returns the raw JSON response.
    ///
    /// # Errors
    ///
    /// Propagates serialization and JSON errors.
    pub fn emulate_transaction_raw(
        &mut self,
        shard_account: &ShardAccount,
        message: &Message,
    ) -> Result<serde_json::Value, EmulatorError> {
        let shard_account = c_string(
            &boc_to_base64(&shard_account.to_cell()?, false)?,
            "shard_account_boc",
        )?;
        let message = c_string(&boc_to_base64(&message.to_cell()?, false)?, "message_boc")?;

        // SAFETY: `handle` is live and both C strings are valid NUL-terminated
        // base64 BoCs for the duration of the call.
        let pointer = unsafe {
            (self.engine.ffi.transaction_emulator_emulate_transaction)(
                self.handle,
                shard_account.as_ptr(),
                message.as_ptr(),
            )
        };
        let response = self.engine.copy_string(pointer)?;
        Ok(serde_json::from_str(&response)?)
    }

    /// Emulates a tick or tock transaction.
    ///
    /// # Errors
    ///
    /// Propagates serialization errors and returns
    /// [`EmulatorError::Emulation`] with the upstream failure details when the
    /// transaction cannot be emulated.
    pub fn emulate_tick_tock_transaction(
        &mut self,
        shard_account: &ShardAccount,
        is_tock: bool,
    ) -> Result<TransactionEmulationResult, EmulatorError> {
        parse_transaction_result(&self.emulate_tick_tock_transaction_raw(shard_account, is_tock)?)
    }

    /// Emulates a tick or tock transaction and returns the raw JSON response.
    ///
    /// # Errors
    ///
    /// Propagates serialization and JSON errors.
    pub fn emulate_tick_tock_transaction_raw(
        &mut self,
        shard_account: &ShardAccount,
        is_tock: bool,
    ) -> Result<serde_json::Value, EmulatorError> {
        let shard_account = c_string(
            &boc_to_base64(&shard_account.to_cell()?, false)?,
            "shard_account_boc",
        )?;

        // SAFETY: `handle` is live and the C string is a valid NUL-terminated
        // base64 BoC for the duration of the call.
        let pointer = unsafe {
            (self
                .engine
                .ffi
                .transaction_emulator_emulate_tick_tock_transaction)(
                self.handle,
                shard_account.as_ptr(),
                is_tock,
            )
        };
        let response = self.engine.copy_string(pointer)?;
        Ok(serde_json::from_str(&response)?)
    }
}

impl Drop for TransactionEmulator {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `handle` was created by `transaction_emulator_create`
            // from this engine and is destroyed exactly once here.
            unsafe { (self.engine.ffi.transaction_emulator_destroy)(self.handle) };
        }
    }
}

/// Builder for [`TransactionEmulator`].
///
/// Every upstream transaction-emulator setter is available before creation:
///
/// ```
/// use tonutils::emulator::{BlockchainConfig, TransactionEmulator, VmLogVerbosity};
/// # fn demo(config: BlockchainConfig) -> Result<(), Box<dyn std::error::Error>> {
/// let mut emulator = TransactionEmulator::builder(config)
///     .vm_log_verbosity(VmLogVerbosity::Unlimited)
///     .unixtime(1_700_000_000)
///     .lt(42_000_000)
///     .rand_seed([0; 32])
///     .ignore_chksig(true)
///     .debug_enabled(true)
///     .build()?;
/// # let _ = &mut emulator;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[must_use = "build the emulator before dropping the builder"]
pub struct TransactionEmulatorBuilder {
    engine: Option<Arc<EmulatorEngine>>,
    config: BlockchainConfig,
    vm_log_verbosity: VmLogVerbosity,
    unixtime: Option<u32>,
    lt: Option<u64>,
    rand_seed: Option<[u8; 32]>,
    ignore_chksig: Option<bool>,
    libraries: Option<Arc<Cell>>,
    debug_enabled: Option<bool>,
    prev_blocks_info: Option<VmStackValue>,
}

impl TransactionEmulatorBuilder {
    /// Creates a builder using the default emulator library and log verbosity.
    pub fn new(config: BlockchainConfig) -> Self {
        Self {
            engine: None,
            config,
            vm_log_verbosity: VmLogVerbosity::Truncated,
            unixtime: None,
            lt: None,
            rand_seed: None,
            ignore_chksig: None,
            libraries: None,
            debug_enabled: None,
            prev_blocks_info: None,
        }
    }

    /// Uses a shared, already-loaded emulator engine.
    pub fn engine(mut self, engine: Arc<EmulatorEngine>) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Sets VM log verbosity for the new emulator.
    pub fn vm_log_verbosity(mut self, verbosity: VmLogVerbosity) -> Self {
        self.vm_log_verbosity = verbosity;
        self
    }

    /// Sets the emulated Unix timestamp.
    pub fn unixtime(mut self, unixtime: u32) -> Self {
        self.unixtime = Some(unixtime);
        self
    }

    /// Sets the emulated logical time.
    pub fn lt(mut self, lt: u64) -> Self {
        self.lt = Some(lt);
        self
    }

    /// Sets the deterministic random seed.
    pub fn rand_seed(mut self, seed: [u8; 32]) -> Self {
        self.rand_seed = Some(seed);
        self
    }

    /// Controls whether CHKSIG operations succeed unconditionally.
    pub fn ignore_chksig(mut self, ignore: bool) -> Self {
        self.ignore_chksig = Some(ignore);
        self
    }

    /// Sets the shared-library dictionary.
    pub fn libraries(mut self, libraries: Arc<Cell>) -> Self {
        self.libraries = Some(libraries);
        self
    }

    /// Enables or disables TVM debug primitives.
    pub fn debug_enabled(mut self, enabled: bool) -> Self {
        self.debug_enabled = Some(enabled);
        self
    }

    /// Sets previous-block info as the 13th `c7` element.
    pub fn prev_blocks_info(mut self, info: VmStackValue) -> Self {
        self.prev_blocks_info = Some(info);
        self
    }

    /// Creates the emulator and applies configured setters.
    ///
    /// # Errors
    ///
    /// Propagates library loading, serialization, creation, and setter errors.
    pub fn build(self) -> Result<TransactionEmulator, EmulatorError> {
        let engine = match self.engine {
            Some(engine) => engine,
            None => EmulatorEngine::open_default()?,
        };
        let config = c_string(&self.config.to_base64()?, "config_params_boc")?;

        // SAFETY: `engine` keeps the library loaded, `config` is a valid
        // NUL-terminated base64 BoC, and the returned pointer is checked before
        // use.
        let handle = unsafe {
            (engine.ffi.transaction_emulator_create)(
                config.as_ptr(),
                self.vm_log_verbosity.as_i32(),
            )
        };
        if handle.is_null() {
            return Err(EmulatorError::CreationFailed {
                kind: "transaction emulator",
            });
        }

        let mut emulator = TransactionEmulator { engine, handle };
        if let Some(value) = self.unixtime {
            emulator.set_unixtime(value)?;
        }
        if let Some(value) = self.lt {
            emulator.set_lt(value)?;
        }
        if let Some(value) = self.rand_seed {
            emulator.set_rand_seed(value)?;
        }
        if let Some(value) = self.ignore_chksig {
            emulator.set_ignore_chksig(value)?;
        }
        if let Some(value) = self.libraries.as_ref() {
            emulator.set_libraries(Some(value))?;
        }
        if let Some(value) = self.debug_enabled {
            emulator.set_debug_enabled(value)?;
        }
        if let Some(value) = self.prev_blocks_info.as_ref() {
            emulator.set_prev_blocks_info(Some(value))?;
        }
        Ok(emulator)
    }
}
