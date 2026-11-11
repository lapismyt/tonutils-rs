//! Standalone TVM emulation and get-method execution.

use std::collections::BTreeMap;
use std::sync::Arc;

use num_bigint::BigUint;

use super::c_string;
use super::config::BlockchainConfig;
use super::engine::EmulatorEngine;
use super::error::EmulatorError;
use super::types::{
    GetMethodResult, MessageEmulationResult, VmLogVerbosity, parse_get_method_result,
    parse_message_result,
};
use crate::tvm::{Address, Cell, VmStack, VmStackValue, boc::boc_to_base64, method_name_to_id};

/// Smart-contract `c7` parameters for standalone TVM emulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct C7Config {
    /// Smart-contract address.
    pub address: Address,
    /// Current Unix timestamp.
    pub unixtime: u32,
    /// Balance in nanotons.
    pub balance: u64,
    /// Deterministic random seed.
    pub rand_seed: [u8; 32],
    /// Optional blockchain configuration.
    pub config: Option<BlockchainConfig>,
}

/// A `libemulator` TVM emulator for get-methods and message execution.
///
/// The value owns an opaque upstream object and is intentionally neither
/// `Send` nor `Sync`; create one per task or thread.
#[derive(Debug)]
pub struct TvmEmulator {
    engine: Arc<EmulatorEngine>,
    handle: *mut std::ffi::c_void,
}

impl TvmEmulator {
    /// Creates a TVM emulator with the default library and log verbosity.
    ///
    /// # Errors
    ///
    /// Propagates library loading and emulator creation errors.
    pub fn new(code: &Arc<Cell>, data: &Arc<Cell>) -> Result<Self, EmulatorError> {
        Self::builder(code.clone(), data.clone()).build()
    }

    /// Creates a builder for a fully configured TVM emulator.
    pub fn builder(code: Arc<Cell>, data: Arc<Cell>) -> TvmEmulatorBuilder {
        TvmEmulatorBuilder::new(code, data)
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
        if unsafe { (self.engine.ffi.tvm_emulator_set_libraries)(self.handle, encoded.as_ptr()) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_libraries",
            })
        }
    }

    /// Sets the smart-contract `c7` parameters.
    ///
    /// # Errors
    ///
    /// Propagates serialization errors and
    /// [`EmulatorError::Rejected`] when the library rejects the parameters.
    pub fn set_c7(&mut self, c7: &C7Config) -> Result<(), EmulatorError> {
        let address = c_string(&c7.address.to_raw(), "c7_address")?;
        let seed = c_string(&hex::encode(c7.rand_seed), "c7_rand_seed")?;
        let config = match &c7.config {
            Some(config) => Some(c_string(&config.to_base64()?, "c7_config")?),
            None => None,
        };

        // SAFETY: `handle` is live and all C strings are valid NUL-terminated
        // inputs matching the upstream contract for the call duration.
        if unsafe {
            (self.engine.ffi.tvm_emulator_set_c7)(
                self.handle,
                address.as_ptr(),
                c7.unixtime,
                c7.balance,
                seed.as_ptr(),
                config
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
            )
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_c7",
            })
        }
    }

    /// Sets extra-currency balances.
    ///
    /// Values are formatted as the upstream space-separated
    /// `currency_id=balance` list, ordered by currency id.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the map.
    pub fn set_extra_currencies(
        &mut self,
        currencies: &BTreeMap<u32, BigUint>,
    ) -> Result<(), EmulatorError> {
        let encoded = currencies
            .iter()
            .map(|(id, balance)| format!("{id}={}", balance.to_str_radix(10)))
            .collect::<Vec<_>>()
            .join(" ");
        let encoded = c_string(&encoded, "extra_currencies")?;

        // SAFETY: `handle` is live and `encoded` is a NUL-terminated upstream
        // extra-currency list.
        if unsafe {
            (self.engine.ffi.tvm_emulator_set_extra_currencies)(self.handle, encoded.as_ptr())
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_extra_currencies",
            })
        }
    }

    /// Sets the previous-block info tuple used as the 13th `c7` element.
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
            (self.engine.ffi.tvm_emulator_set_prev_blocks_info)(
                self.handle,
                encoded
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
            )
        } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_prev_blocks_info",
            })
        }
    }

    /// Sets the TVM gas limit.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the value.
    pub fn set_gas_limit(&mut self, gas_limit: i64) -> Result<(), EmulatorError> {
        // SAFETY: `handle` is live and the setter accepts an `int64_t`.
        if unsafe { (self.engine.ffi.tvm_emulator_set_gas_limit)(self.handle, gas_limit) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_gas_limit",
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
        if unsafe { (self.engine.ffi.tvm_emulator_set_debug_enabled)(self.handle, enabled) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "tvm_emulator_set_debug_enabled",
            })
        }
    }

    /// Runs a get-method by numeric method id.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::InvalidInput`] when the method id does not fit
    /// a signed 32-bit upstream `int`, and propagates stack and emulation
    /// errors.
    pub fn run_get_method(
        &mut self,
        method_id: u32,
        stack: &VmStack,
    ) -> Result<GetMethodResult, EmulatorError> {
        parse_get_method_result(&self.run_get_method_raw(method_id, stack)?)
    }

    /// Runs a get-method by name.
    ///
    /// # Errors
    ///
    /// Propagates method-id conversion, stack, and emulation errors.
    pub fn run_get_method_by_name(
        &mut self,
        method_name: &str,
        stack: &VmStack,
    ) -> Result<GetMethodResult, EmulatorError> {
        let method_id = u32::try_from(method_name_to_id(method_name)).map_err(|_| {
            EmulatorError::InvalidInput(format!(
                "get-method id for `{method_name}` does not fit u32"
            ))
        })?;
        self.run_get_method(method_id, stack)
    }

    /// Runs a get-method and returns the raw JSON response.
    ///
    /// # Errors
    ///
    /// Propagates serialization and JSON errors.
    pub fn run_get_method_raw(
        &mut self,
        method_id: u32,
        stack: &VmStack,
    ) -> Result<serde_json::Value, EmulatorError> {
        let method_id = i32::try_from(method_id).map_err(|_| {
            EmulatorError::InvalidInput(format!(
                "get-method id {method_id} does not fit a signed 32-bit integer"
            ))
        })?;
        let encoded = c_string(&boc_to_base64(&stack.to_cell()?, false)?, "stack_boc")?;

        // SAFETY: `handle` is live and `encoded` is a NUL-terminated base64 BoC
        // of a standard VmStack.
        let pointer = unsafe {
            (self.engine.ffi.tvm_emulator_run_get_method)(self.handle, method_id, encoded.as_ptr())
        };
        let response = self.engine.copy_string(pointer)?;
        Ok(serde_json::from_str(&response)?)
    }

    /// Sends an external message body to the emulated contract.
    ///
    /// # Errors
    ///
    /// Propagates serialization and emulation errors.
    pub fn send_external_message(
        &mut self,
        message_body: &Arc<Cell>,
    ) -> Result<MessageEmulationResult, EmulatorError> {
        parse_message_result(&self.send_external_message_raw(message_body)?)
    }

    /// Sends an external message body and returns the raw JSON response.
    ///
    /// # Errors
    ///
    /// Propagates serialization and JSON errors.
    pub fn send_external_message_raw(
        &mut self,
        message_body: &Arc<Cell>,
    ) -> Result<serde_json::Value, EmulatorError> {
        let encoded = c_string(&boc_to_base64(message_body, false)?, "message_body_boc")?;

        // SAFETY: `handle` is live and `encoded` is a NUL-terminated base64 BoC.
        let pointer = unsafe {
            (self.engine.ffi.tvm_emulator_send_external_message)(self.handle, encoded.as_ptr())
        };
        let response = self.engine.copy_string(pointer)?;
        Ok(serde_json::from_str(&response)?)
    }

    /// Sends an internal message body with the attached amount in nanotons.
    ///
    /// # Errors
    ///
    /// Propagates serialization and emulation errors.
    pub fn send_internal_message(
        &mut self,
        message_body: &Arc<Cell>,
        amount: u64,
    ) -> Result<MessageEmulationResult, EmulatorError> {
        parse_message_result(&self.send_internal_message_raw(message_body, amount)?)
    }

    /// Sends an internal message body and returns the raw JSON response.
    ///
    /// # Errors
    ///
    /// Propagates serialization and JSON errors.
    pub fn send_internal_message_raw(
        &mut self,
        message_body: &Arc<Cell>,
        amount: u64,
    ) -> Result<serde_json::Value, EmulatorError> {
        let encoded = c_string(&boc_to_base64(message_body, false)?, "message_body_boc")?;

        // SAFETY: `handle` is live and `encoded` is a NUL-terminated base64 BoC.
        let pointer = unsafe {
            (self.engine.ffi.tvm_emulator_send_internal_message)(
                self.handle,
                encoded.as_ptr(),
                amount,
            )
        };
        let response = self.engine.copy_string(pointer)?;
        Ok(serde_json::from_str(&response)?)
    }
}

impl Drop for TvmEmulator {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `handle` was created by `tvm_emulator_create` from this
            // engine and is destroyed exactly once here.
            unsafe { (self.engine.ffi.tvm_emulator_destroy)(self.handle) };
        }
    }
}

/// Builder for [`TvmEmulator`].
///
/// ```
/// use std::collections::BTreeMap;
/// use std::sync::Arc;
/// use tonutils::emulator::{C7Config, TvmEmulator, VmLogVerbosity};
/// use tonutils::tvm::{Address, Cell};
/// # fn demo(code: Arc<Cell>, data: Arc<Cell>, address: Address) -> Result<(), Box<dyn std::error::Error>> {
/// let mut emulator = TvmEmulator::builder(code, data)
///     .vm_log_verbosity(VmLogVerbosity::Unlimited)
///     .c7(C7Config {
///         address,
///         unixtime: 1_700_000_000,
///         balance: 1_000_000_000,
///         rand_seed: [0; 32],
///         config: None,
///     })
///     .extra_currencies(BTreeMap::new())
///     .gas_limit(1_000_000)
///     .debug_enabled(true)
///     .build()?;
/// # let _ = &mut emulator;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[must_use = "build the emulator before dropping the builder"]
pub struct TvmEmulatorBuilder {
    engine: Option<Arc<EmulatorEngine>>,
    code: Arc<Cell>,
    data: Arc<Cell>,
    vm_log_verbosity: VmLogVerbosity,
    libraries: Option<Arc<Cell>>,
    c7: Option<C7Config>,
    extra_currencies: Option<BTreeMap<u32, BigUint>>,
    prev_blocks_info: Option<VmStackValue>,
    gas_limit: Option<i64>,
    debug_enabled: Option<bool>,
}

impl TvmEmulatorBuilder {
    /// Creates a builder using the default emulator library and log verbosity.
    pub fn new(code: Arc<Cell>, data: Arc<Cell>) -> Self {
        Self {
            engine: None,
            code,
            data,
            vm_log_verbosity: VmLogVerbosity::Truncated,
            libraries: None,
            c7: None,
            extra_currencies: None,
            prev_blocks_info: None,
            gas_limit: None,
            debug_enabled: None,
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

    /// Sets the shared-library dictionary.
    pub fn libraries(mut self, libraries: Arc<Cell>) -> Self {
        self.libraries = Some(libraries);
        self
    }

    /// Sets the smart-contract `c7` parameters.
    pub fn c7(mut self, c7: C7Config) -> Self {
        self.c7 = Some(c7);
        self
    }

    /// Sets extra-currency balances.
    pub fn extra_currencies(mut self, currencies: BTreeMap<u32, BigUint>) -> Self {
        self.extra_currencies = Some(currencies);
        self
    }

    /// Sets previous-block info as the 13th `c7` element.
    pub fn prev_blocks_info(mut self, info: VmStackValue) -> Self {
        self.prev_blocks_info = Some(info);
        self
    }

    /// Sets the TVM gas limit.
    pub fn gas_limit(mut self, gas_limit: i64) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Enables or disables TVM debug primitives.
    pub fn debug_enabled(mut self, enabled: bool) -> Self {
        self.debug_enabled = Some(enabled);
        self
    }

    /// Creates the emulator and applies configured setters.
    ///
    /// # Errors
    ///
    /// Propagates library loading, serialization, creation, and setter errors.
    pub fn build(self) -> Result<TvmEmulator, EmulatorError> {
        let engine = match self.engine {
            Some(engine) => engine,
            None => EmulatorEngine::open_default()?,
        };
        let code = c_string(&boc_to_base64(&self.code, false)?, "code_boc")?;
        let data = c_string(&boc_to_base64(&self.data, false)?, "data_boc")?;

        // SAFETY: `engine` keeps the library loaded, both C strings are valid
        // NUL-terminated base64 BoCs, and the returned pointer is checked.
        let handle = unsafe {
            (engine.ffi.tvm_emulator_create)(
                code.as_ptr(),
                data.as_ptr(),
                self.vm_log_verbosity.as_i32(),
            )
        };
        if handle.is_null() {
            return Err(EmulatorError::CreationFailed {
                kind: "TVM emulator",
            });
        }

        let mut emulator = TvmEmulator { engine, handle };
        if let Some(value) = self.libraries.as_ref() {
            emulator.set_libraries(Some(value))?;
        }
        if let Some(value) = self.c7.as_ref() {
            emulator.set_c7(value)?;
        }
        if let Some(value) = self.extra_currencies.as_ref() {
            emulator.set_extra_currencies(value)?;
        }
        if let Some(value) = self.prev_blocks_info.as_ref() {
            emulator.set_prev_blocks_info(Some(value))?;
        }
        if let Some(value) = self.gas_limit {
            emulator.set_gas_limit(value)?;
        }
        if let Some(value) = self.debug_enabled {
            emulator.set_debug_enabled(value)?;
        }
        Ok(emulator)
    }
}
