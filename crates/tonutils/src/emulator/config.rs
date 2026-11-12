//! Blockchain configuration inputs for emulation.

use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;

use crate::tlb::{ConfigParams, TlbDeserialize};
use crate::tvm::{Cell, boc::boc_to_base64, boc::deserialize_boc};

use super::error::EmulatorError;

/// The `Hashmap 32 ^Cell` blockchain configuration dictionary.
///
/// `libemulator` accepts the configuration dictionary root, **not** the outer
/// [`ConfigParams`] cell. Use [`from_config_params`](Self::from_config_params)
/// to extract the correct root from a typed config wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockchainConfig {
    dictionary: Arc<Cell>,
}

impl BlockchainConfig {
    /// Wraps an already-decoded configuration dictionary root.
    pub fn from_dictionary(dictionary: Arc<Cell>) -> Self {
        Self { dictionary }
    }

    /// Extracts the configuration dictionary root from typed config params.
    pub fn from_config_params(params: &ConfigParams) -> Self {
        Self::from_dictionary(params.config.clone())
    }

    /// Decodes a configuration dictionary root from BoC bytes.
    ///
    /// # Errors
    ///
    /// Propagates BoC decoding errors.
    pub fn from_dictionary_boc(bytes: &[u8]) -> Result<Self, EmulatorError> {
        Ok(Self::from_dictionary(deserialize_boc(bytes)?))
    }

    /// Decodes an outer [`ConfigParams`] cell from BoC bytes and extracts its
    /// configuration dictionary root.
    ///
    /// # Errors
    ///
    /// Propagates BoC and TL-B decoding errors.
    pub fn from_config_params_boc(bytes: &[u8]) -> Result<Self, EmulatorError> {
        let cell = deserialize_boc(bytes)?;
        let params = ConfigParams::from_cell(cell)?;
        Ok(Self::from_config_params(&params))
    }

    /// Reads configuration from a file containing either raw BoC bytes or
    /// base64-encoded BoC text.
    ///
    /// # Errors
    ///
    /// Returns [`EmulatorError::Io`] when the file cannot be read and
    /// propagates BoC decoding errors.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, EmulatorError> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Decodes configuration from either raw BoC bytes or base64 text.
    ///
    /// # Errors
    ///
    /// Propagates base64 or BoC decoding errors.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EmulatorError> {
        let trimmed = trim_ascii(bytes);
        if !trimmed.is_empty()
            && trimmed
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        {
            let decoded = B64_STANDARD.decode(trimmed)?;
            return Self::from_dictionary_boc(&decoded);
        }
        Self::from_dictionary_boc(bytes)
    }

    /// Returns the configuration dictionary root cell.
    pub fn dictionary(&self) -> &Arc<Cell> {
        &self.dictionary
    }

    /// Serializes the configuration dictionary root to BoC bytes.
    ///
    /// # Errors
    ///
    /// Propagates BoC serialization errors.
    pub fn to_boc(&self) -> Result<Vec<u8>, EmulatorError> {
        crate::tvm::serialize_boc(&self.dictionary, false).map_err(EmulatorError::from)
    }

    /// Serializes the configuration dictionary root to base64 BoC text.
    ///
    /// # Errors
    ///
    /// Propagates BoC serialization errors.
    pub fn to_base64(&self) -> Result<String, EmulatorError> {
        boc_to_base64(&self.dictionary, false).map_err(EmulatorError::from)
    }
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}
