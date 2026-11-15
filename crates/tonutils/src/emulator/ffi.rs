//! Raw `libemulator` ABI declarations.
//!
//! The surface mirrors `emulator/emulator-extern.h` from
//! `ton-blockchain/ton`. All symbol loading and calls are confined to this
//! module and the safe wrappers that use it.

use std::ffi::{c_char, c_int, c_void};
use std::path::Path;

use libloading::Library;

use super::error::EmulatorError;

pub(crate) type TransactionEmulatorCreate =
    unsafe extern "C" fn(*const c_char, c_int) -> *mut c_void;
pub(crate) type TransactionEmulatorSetUnixtime = unsafe extern "C" fn(*mut c_void, u32) -> bool;
pub(crate) type TransactionEmulatorSetLt = unsafe extern "C" fn(*mut c_void, u64) -> bool;
pub(crate) type TransactionEmulatorSetRandSeed =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TransactionEmulatorSetIgnoreChksig =
    unsafe extern "C" fn(*mut c_void, bool) -> bool;
pub(crate) type TransactionEmulatorSetConfig =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TransactionEmulatorSetConfigObject =
    unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
pub(crate) type TransactionEmulatorSetLibs =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TransactionEmulatorSetDebugEnabled =
    unsafe extern "C" fn(*mut c_void, bool) -> bool;
pub(crate) type TransactionEmulatorSetPrevBlocksInfo =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TransactionEmulatorEmulateTransaction =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *const c_char;
pub(crate) type TransactionEmulatorEmulateTickTock =
    unsafe extern "C" fn(*mut c_void, *const c_char, bool) -> *const c_char;
pub(crate) type TransactionEmulatorDestroy = unsafe extern "C" fn(*mut c_void);
pub(crate) type EmulatorSetVerbosityLevel = unsafe extern "C" fn(c_int) -> bool;
pub(crate) type TvmEmulatorCreate =
    unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> *mut c_void;
pub(crate) type TvmEmulatorSetLibraries = unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TvmEmulatorSetC7 = unsafe extern "C" fn(
    *mut c_void,
    *const c_char,
    u32,
    u64,
    *const c_char,
    *const c_char,
) -> bool;
pub(crate) type TvmEmulatorSetExtraCurrencies =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TvmEmulatorSetPrevBlocksInfo =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> bool;
pub(crate) type TvmEmulatorSetConfigObject = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
pub(crate) type TvmEmulatorSetGasLimit = unsafe extern "C" fn(*mut c_void, i64) -> bool;
pub(crate) type TvmEmulatorSetDebugEnabled = unsafe extern "C" fn(*mut c_void, bool) -> bool;
pub(crate) type TvmEmulatorEmulateRunMethod =
    unsafe extern "C" fn(u32, *const c_char, i64) -> *const c_char;
pub(crate) type TvmEmulatorEmulateRunMethodDetailed =
    unsafe extern "C" fn(u32, *const c_char, i64) -> *mut c_void;
pub(crate) type TvmEmulatorRunGetMethod =
    unsafe extern "C" fn(*mut c_void, c_int, *const c_char) -> *const c_char;
pub(crate) type TvmEmulatorSendExternalMessage =
    unsafe extern "C" fn(*mut c_void, *const c_char) -> *const c_char;
pub(crate) type TvmEmulatorSendInternalMessage =
    unsafe extern "C" fn(*mut c_void, *const c_char, u64) -> *const c_char;
pub(crate) type TvmEmulatorDestroy = unsafe extern "C" fn(*mut c_void);
pub(crate) type RunMethodDetailedResultDestroy = unsafe extern "C" fn(*mut c_void);
pub(crate) type EmulatorConfigCreate = unsafe extern "C" fn(*const c_char) -> *mut c_void;
pub(crate) type EmulatorConfigDestroy = unsafe extern "C" fn(*mut c_void);
pub(crate) type StringDestroy = unsafe extern "C" fn(*const c_char);
pub(crate) type EmulatorVersion = unsafe extern "C" fn() -> *const c_char;

/// Loaded dynamic library and every symbol used by the safe wrappers.
pub(crate) struct EmulatorFfi {
    _library: Library,
    pub(crate) transaction_emulator_create: TransactionEmulatorCreate,
    pub(crate) transaction_emulator_set_unixtime: TransactionEmulatorSetUnixtime,
    pub(crate) transaction_emulator_set_lt: TransactionEmulatorSetLt,
    pub(crate) transaction_emulator_set_rand_seed: TransactionEmulatorSetRandSeed,
    pub(crate) transaction_emulator_set_ignore_chksig: TransactionEmulatorSetIgnoreChksig,
    pub(crate) transaction_emulator_set_config: TransactionEmulatorSetConfig,
    pub(crate) transaction_emulator_set_config_object: TransactionEmulatorSetConfigObject,
    pub(crate) transaction_emulator_set_libs: TransactionEmulatorSetLibs,
    pub(crate) transaction_emulator_set_debug_enabled: TransactionEmulatorSetDebugEnabled,
    pub(crate) transaction_emulator_set_prev_blocks_info: TransactionEmulatorSetPrevBlocksInfo,
    pub(crate) transaction_emulator_emulate_transaction: TransactionEmulatorEmulateTransaction,
    pub(crate) transaction_emulator_emulate_tick_tock_transaction:
        TransactionEmulatorEmulateTickTock,
    pub(crate) transaction_emulator_destroy: TransactionEmulatorDestroy,
    pub(crate) emulator_set_verbosity_level: EmulatorSetVerbosityLevel,
    pub(crate) tvm_emulator_create: TvmEmulatorCreate,
    pub(crate) tvm_emulator_set_libraries: TvmEmulatorSetLibraries,
    pub(crate) tvm_emulator_set_c7: TvmEmulatorSetC7,
    pub(crate) tvm_emulator_set_extra_currencies: TvmEmulatorSetExtraCurrencies,
    pub(crate) tvm_emulator_set_prev_blocks_info: TvmEmulatorSetPrevBlocksInfo,
    pub(crate) tvm_emulator_set_config_object: TvmEmulatorSetConfigObject,
    pub(crate) tvm_emulator_set_gas_limit: TvmEmulatorSetGasLimit,
    pub(crate) tvm_emulator_set_debug_enabled: TvmEmulatorSetDebugEnabled,
    pub(crate) tvm_emulator_emulate_run_method: TvmEmulatorEmulateRunMethod,
    pub(crate) tvm_emulator_emulate_run_method_detailed: TvmEmulatorEmulateRunMethodDetailed,
    pub(crate) run_method_detailed_result_destroy: RunMethodDetailedResultDestroy,
    pub(crate) tvm_emulator_run_get_method: TvmEmulatorRunGetMethod,
    pub(crate) tvm_emulator_send_external_message: TvmEmulatorSendExternalMessage,
    pub(crate) tvm_emulator_send_internal_message: TvmEmulatorSendInternalMessage,
    pub(crate) tvm_emulator_destroy: TvmEmulatorDestroy,
    pub(crate) emulator_config_create: EmulatorConfigCreate,
    pub(crate) emulator_config_destroy: EmulatorConfigDestroy,
    pub(crate) string_destroy: StringDestroy,
    pub(crate) emulator_version: EmulatorVersion,
}

macro_rules! load_symbol {
    ($library:expr, $name:literal, $ty:ty) => {{
        let symbol =
            unsafe { $library.get::<$ty>(concat!($name, "\0").as_bytes()) }.map_err(|source| {
                EmulatorError::MissingSymbol {
                    symbol: $name.to_owned(),
                    source,
                }
            })?;
        *symbol
    }};
}

impl EmulatorFfi {
    /// Loads a specific `libemulator` path.
    #[expect(
        clippy::too_many_lines,
        reason = "The loader mirrors the upstream C header symbol-by-symbol"
    )]
    pub(crate) fn load(path: &Path) -> Result<Self, EmulatorError> {
        // SAFETY: the caller asks for a dynamic library to be mapped into the
        // process. Symbol calls are validated by their declared C signatures
        // and only occur through the safe wrappers in this crate.
        let library = unsafe { Library::new(path) }.map_err(|source| {
            EmulatorError::LibraryOpen(format!("{}: {source}", path.display()))
        })?;

        Ok(Self {
            transaction_emulator_create: load_symbol!(
                library,
                "transaction_emulator_create",
                TransactionEmulatorCreate
            ),
            transaction_emulator_set_unixtime: load_symbol!(
                library,
                "transaction_emulator_set_unixtime",
                TransactionEmulatorSetUnixtime
            ),
            transaction_emulator_set_lt: load_symbol!(
                library,
                "transaction_emulator_set_lt",
                TransactionEmulatorSetLt
            ),
            transaction_emulator_set_rand_seed: load_symbol!(
                library,
                "transaction_emulator_set_rand_seed",
                TransactionEmulatorSetRandSeed
            ),
            transaction_emulator_set_ignore_chksig: load_symbol!(
                library,
                "transaction_emulator_set_ignore_chksig",
                TransactionEmulatorSetIgnoreChksig
            ),
            transaction_emulator_set_config: load_symbol!(
                library,
                "transaction_emulator_set_config",
                TransactionEmulatorSetConfig
            ),
            transaction_emulator_set_config_object: load_symbol!(
                library,
                "transaction_emulator_set_config_object",
                TransactionEmulatorSetConfigObject
            ),
            transaction_emulator_set_libs: load_symbol!(
                library,
                "transaction_emulator_set_libs",
                TransactionEmulatorSetLibs
            ),
            transaction_emulator_set_debug_enabled: load_symbol!(
                library,
                "transaction_emulator_set_debug_enabled",
                TransactionEmulatorSetDebugEnabled
            ),
            transaction_emulator_set_prev_blocks_info: load_symbol!(
                library,
                "transaction_emulator_set_prev_blocks_info",
                TransactionEmulatorSetPrevBlocksInfo
            ),
            transaction_emulator_emulate_transaction: load_symbol!(
                library,
                "transaction_emulator_emulate_transaction",
                TransactionEmulatorEmulateTransaction
            ),
            transaction_emulator_emulate_tick_tock_transaction: load_symbol!(
                library,
                "transaction_emulator_emulate_tick_tock_transaction",
                TransactionEmulatorEmulateTickTock
            ),
            transaction_emulator_destroy: load_symbol!(
                library,
                "transaction_emulator_destroy",
                TransactionEmulatorDestroy
            ),
            emulator_set_verbosity_level: load_symbol!(
                library,
                "emulator_set_verbosity_level",
                EmulatorSetVerbosityLevel
            ),
            tvm_emulator_create: load_symbol!(library, "tvm_emulator_create", TvmEmulatorCreate),
            tvm_emulator_set_libraries: load_symbol!(
                library,
                "tvm_emulator_set_libraries",
                TvmEmulatorSetLibraries
            ),
            tvm_emulator_set_c7: load_symbol!(library, "tvm_emulator_set_c7", TvmEmulatorSetC7),
            tvm_emulator_set_extra_currencies: load_symbol!(
                library,
                "tvm_emulator_set_extra_currencies",
                TvmEmulatorSetExtraCurrencies
            ),
            tvm_emulator_set_prev_blocks_info: load_symbol!(
                library,
                "tvm_emulator_set_prev_blocks_info",
                TvmEmulatorSetPrevBlocksInfo
            ),
            tvm_emulator_set_config_object: load_symbol!(
                library,
                "tvm_emulator_set_config_object",
                TvmEmulatorSetConfigObject
            ),
            tvm_emulator_set_gas_limit: load_symbol!(
                library,
                "tvm_emulator_set_gas_limit",
                TvmEmulatorSetGasLimit
            ),
            tvm_emulator_set_debug_enabled: load_symbol!(
                library,
                "tvm_emulator_set_debug_enabled",
                TvmEmulatorSetDebugEnabled
            ),
            tvm_emulator_emulate_run_method: load_symbol!(
                library,
                "tvm_emulator_emulate_run_method",
                TvmEmulatorEmulateRunMethod
            ),
            tvm_emulator_emulate_run_method_detailed: load_symbol!(
                library,
                "tvm_emulator_emulate_run_method_detailed",
                TvmEmulatorEmulateRunMethodDetailed
            ),
            run_method_detailed_result_destroy: load_symbol!(
                library,
                "run_method_detailed_result_destroy",
                RunMethodDetailedResultDestroy
            ),
            tvm_emulator_run_get_method: load_symbol!(
                library,
                "tvm_emulator_run_get_method",
                TvmEmulatorRunGetMethod
            ),
            tvm_emulator_send_external_message: load_symbol!(
                library,
                "tvm_emulator_send_external_message",
                TvmEmulatorSendExternalMessage
            ),
            tvm_emulator_send_internal_message: load_symbol!(
                library,
                "tvm_emulator_send_internal_message",
                TvmEmulatorSendInternalMessage
            ),
            tvm_emulator_destroy: load_symbol!(library, "tvm_emulator_destroy", TvmEmulatorDestroy),
            emulator_config_create: load_symbol!(
                library,
                "emulator_config_create",
                EmulatorConfigCreate
            ),
            emulator_config_destroy: load_symbol!(
                library,
                "emulator_config_destroy",
                EmulatorConfigDestroy
            ),
            string_destroy: load_symbol!(library, "string_destroy", StringDestroy),
            emulator_version: load_symbol!(library, "emulator_version", EmulatorVersion),
            _library: library,
        })
    }
}
