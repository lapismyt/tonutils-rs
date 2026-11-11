//! Errors returned by the emulator feature.

use thiserror::Error;

use crate::tlb::TlbError;
use crate::tvm::TvmError;

/// Errors raised while loading or using `libemulator`.
#[derive(Debug, Error)]
pub enum EmulatorError {
    /// The dynamic emulator library could not be opened.
    #[error("failed to open libemulator: {0}")]
    LibraryOpen(String),

    /// The dynamic emulator library does not export a required symbol.
    #[error("libemulator is missing symbol {symbol}: {source}")]
    MissingSymbol {
        /// Missing symbol name.
        symbol: String,
        /// Dynamic loader error.
        #[source]
        source: libloading::Error,
    },

    /// The library returned a null pointer where an object was expected.
    #[error("libemulator failed to create {kind}")]
    CreationFailed {
        /// Object kind that could not be created.
        kind: &'static str,
    },

    /// An upstream setter returned `false`.
    #[error("libemulator rejected {operation}")]
    Rejected {
        /// Setter or operation name.
        operation: &'static str,
    },

    /// Emulation failed and the library reported the reason in JSON.
    #[error("emulation failed: {message}")]
    Emulation {
        /// Human-readable error message.
        message: String,
        /// Whether the failure was an unaccepted external message.
        external_not_accepted: bool,
        /// Optional VM exit code.
        vm_exit_code: Option<i32>,
        /// Optional VM log.
        vm_log: Option<String>,
        /// Optional elapsed time in seconds.
        elapsed_time: Option<f64>,
    },

    /// The library returned malformed or unexpected JSON.
    #[error("invalid libemulator response: {0}")]
    InvalidResponse(String),

    /// A supplied value is not valid for the requested operation.
    #[error("invalid emulator input: {0}")]
    InvalidInput(String),

    /// The response was not valid JSON.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Base64 decoding failed.
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),

    /// A TON cell, BoC, address, or stack operation failed.
    #[error(transparent)]
    Tvm(#[from] TvmError),

    /// A TL-B model operation failed.
    #[error(transparent)]
    Tlb(#[from] TlbError),

    /// A local file could not be read.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl EmulatorError {
    /// Returns the VM exit code when the failure carries one.
    pub fn vm_exit_code(&self) -> Option<i32> {
        match self {
            Self::Emulation { vm_exit_code, .. } => *vm_exit_code,
            _ => None,
        }
    }

    /// Returns the VM log when the failure carries one.
    pub fn vm_log(&self) -> Option<&str> {
        match self {
            Self::Emulation { vm_log, .. } => vm_log.as_deref(),
            _ => None,
        }
    }

    /// Returns whether an external message was rejected by the smart contract.
    pub fn external_not_accepted(&self) -> bool {
        matches!(
            self,
            Self::Emulation {
                external_not_accepted: true,
                ..
            }
        )
    }
}
