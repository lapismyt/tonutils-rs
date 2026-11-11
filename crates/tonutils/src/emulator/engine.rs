//! Dynamic loading and shared library management for `libemulator`.

use std::env::consts::DLL_EXTENSION;
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::error::EmulatorError;
use super::ffi::EmulatorFfi;

/// Environment variable checked first when locating `libemulator`.
pub const EMULATOR_LIB_ENV: &str = "TONUTILS_EMULATOR_LIB";

/// Secondary environment variable honored when locating `libemulator`.
pub const EMULATOR_LIB_FALLBACK_ENV: &str = "TON_EMULATOR_LIB";

/// Global log level of the emulator library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EmulatorLogLevel {
    /// Disable library logging.
    Never = 0,
    /// Log errors only.
    Error = 1,
    /// Log warnings and errors.
    Warning = 2,
    /// Log informational messages, warnings, and errors.
    Info = 3,
    /// Log debug messages and all lower levels.
    Debug = 4,
}

impl EmulatorLogLevel {
    /// Returns the upstream numeric verbosity value.
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// A loaded `libemulator` instance.
///
/// The engine owns the dynamic library and all resolved function pointers.
/// Emulator objects keep an [`Arc`] of this engine so the library cannot be
/// unloaded while an emulator still exists.
///
/// The engine itself can be shared across threads, but upstream emulator
/// objects are not thread-safe. Use one emulator per task.
pub struct EmulatorEngine {
    path: PathBuf,
    pub(crate) ffi: EmulatorFfi,
}

impl std::fmt::Debug for EmulatorEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmulatorEngine")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl EmulatorEngine {
    /// Loads `libemulator` from an explicit path.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::LibraryOpen`] when the library cannot be
    /// loaded, or [`EmulatorError::MissingSymbol`] when a required upstream
    /// symbol is absent.
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, EmulatorError> {
        let path = path.as_ref().to_path_buf();
        let ffi = EmulatorFfi::load(&path)?;
        Ok(Arc::new(Self { path, ffi }))
    }

    /// Loads `libemulator` from the environment or a platform default name.
    ///
    /// Resolution order:
    ///
    /// 1. `TONUTILS_EMULATOR_LIB`;
    /// 2. `TON_EMULATOR_LIB`;
    /// 3. `libemulator` with the platform dynamic-library extension.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::LibraryOpen`] describing every attempted path
    /// when no candidate can be loaded.
    pub fn open_default() -> Result<Arc<Self>, EmulatorError> {
        for variable in [EMULATOR_LIB_ENV, EMULATOR_LIB_FALLBACK_ENV] {
            if let Some(value) = std::env::var_os(variable)
                && !value.is_empty()
            {
                return Self::open(value);
            }
        }

        let candidate = PathBuf::from(format!("libemulator.{DLL_EXTENSION}"));
        Self::open(&candidate).map_err(|error| {
            EmulatorError::LibraryOpen(format!(
                "{error}; set {EMULATOR_LIB_ENV} or pass an explicit path to EmulatorEngine::open"
            ))
        })
    }

    /// Returns the path this engine loaded.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the emulator library version metadata as JSON text.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::InvalidResponse`] when the library returns a
    /// null string.
    pub fn version(&self) -> Result<String, EmulatorError> {
        // SAFETY: `emulator_version` has no arguments and returns either a
        // null pointer or a NUL-terminated string owned by the library.
        let pointer = unsafe { (self.ffi.emulator_version)() };
        self.copy_string(pointer)
    }

    /// Sets the global emulator library log level.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Rejected`] when the library rejects the level.
    pub fn set_verbosity(&self, level: EmulatorLogLevel) -> Result<(), EmulatorError> {
        // SAFETY: the FFI signature accepts an `int` log level and returns a
        // C `bool`; `level` is a valid enumerated value.
        if unsafe { (self.ffi.emulator_set_verbosity_level)(level.as_i32()) } {
            Ok(())
        } else {
            Err(EmulatorError::Rejected {
                operation: "emulator_set_verbosity_level",
            })
        }
    }

    pub(crate) fn copy_string(
        &self,
        pointer: *const std::ffi::c_char,
    ) -> Result<String, EmulatorError> {
        if pointer.is_null() {
            return Err(EmulatorError::InvalidResponse(
                "libemulator returned a null string".to_owned(),
            ));
        }

        // SAFETY: the library contract requires a NUL-terminated string for
        // every non-null `const char *` response. The string is copied before
        // being released, and `string_destroy` is the matching deallocator.
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `pointer` came from this library and is destroyed exactly
        // once here.
        unsafe { (self.ffi.string_destroy)(pointer) };
        Ok(text)
    }
}
