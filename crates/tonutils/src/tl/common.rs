use core::fmt;
use std::{fmt::Display, str::FromStr};

use derivative::Derivative;
use hex::FromHex;
use tl_proto::{TlRead, TlWrite};

use super::utils::{fmt_bytes, fmt_string};

/// true = True;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct True;

/// string ? = String;
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq)]
pub struct TlString(#[derivative(Debug(format_with = "fmt_string"))] Vec<u8>);

impl Display for TlString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl From<&str> for TlString {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

impl TlString {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value.into_bytes())
    }
}

/// TL schema declaration: `int256 8*[ int ] = Int256;`.
#[derive(TlRead, TlWrite, Derivative)]
#[derivative(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct TlInt256(#[derivative(Debug(format_with = "fmt_bytes"))] pub [u8; 32]);

impl FromStr for TlInt256 {
    type Err = hex::FromHexError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

impl Display for TlInt256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl TlInt256 {
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parses a hexadecimal 256-bit integer.
    ///
    /// # Errors
    ///
    /// Returns [`hex::FromHexError`] when `s` is not exactly 32 bytes of hexadecimal data.
    pub fn from_hex(value: &str) -> Result<Self, hex::FromHexError> {
        Ok(Self(<[u8; 32]>::from_hex(value)?))
    }

    #[must_use]
    pub fn random() -> Self {
        Self(rand::random())
    }
}
