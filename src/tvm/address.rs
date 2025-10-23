//! TON Address implementation
//!
//! Supports both internal addresses (workchain + hash) and external addresses.

use anyhow::{Result, bail};
use base64::Engine;
use std::fmt;
use crate::tl::{common::AccountId, Int256};

/// CRC16 calculation for address validation
fn crc16(data: &[u8]) -> [u8; 2] {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc.to_be_bytes()
}

/// Represents a TON blockchain address
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    /// Workchain ID (-1 for masterchain, 0 for basechain)
    pub workchain: i8,
    /// 32-byte hash part of the address
    pub hash_part: [u8; 32],
    /// Whether the address is bounceable
    pub is_bounceable: bool,
    /// Whether this is a test-only address
    pub is_test_only: bool,
}

impl Address {
    /// Creates a new address from workchain and hash part
    pub fn new(workchain: i8, hash_part: [u8; 32]) -> Self {
        Self {
            workchain,
            hash_part,
            is_bounceable: true,
            is_test_only: false,
        }
    }

    /// Parses an address from string (supports both hex and base64 formats)
    pub fn from_str(address: &str) -> Result<Self> {
        // Try hex format first (faster)
        if let Ok(addr) = Self::from_hex(address) {
            return Ok(addr);
        }

        // Try base64 format
        if let Ok(addr) = Self::from_base64(address) {
            return Ok(addr);
        }

        bail!("Invalid address format")
    }

    /// Parses address from hex format: "workchain:hash"
    pub fn from_hex(address: &str) -> Result<Self> {
        let parts: Vec<&str> = address.split(':').collect();
        if parts.len() != 2 {
            bail!("Invalid hex address format");
        }

        let workchain = parts[0].parse::<i8>()?;
        let hash_hex = parts[1];

        if hash_hex.len() != 64 {
            bail!("Hash part must be 64 hex characters");
        }

        let hash_bytes = hex::decode(hash_hex)?;
        let mut hash_part = [0u8; 32];
        hash_part.copy_from_slice(&hash_bytes);

        Ok(Self::new(workchain, hash_part))
    }

    /// Parses address from base64 user-friendly format
    pub fn from_base64(address: &str) -> Result<Self> {
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(address)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(address))?;

        if decoded.len() != 36 {
            bail!("Invalid base64 address length");
        }

        let mut tag = decoded[0];
        let mut is_test_only = false;

        // Check test flag
        if tag & 0x80 != 0 {
            is_test_only = true;
            tag ^= 0x80;
        }

        let is_bounceable = match tag {
            0x11 => true,  // bounceable
            0x51 => false, // non-bounceable
            _ => bail!("Invalid address tag"),
        };

        let workchain = decoded[1] as i8;
        let mut hash_part = [0u8; 32];
        hash_part.copy_from_slice(&decoded[2..34]);

        // Verify CRC16
        let expected_crc = &decoded[34..36];
        let actual_crc = crc16(&decoded[0..34]);

        if expected_crc != actual_crc {
            bail!("Invalid address CRC");
        }

        Ok(Self {
            workchain,
            hash_part,
            is_bounceable,
            is_test_only,
        })
    }

    /// Converts address to string representation
    pub fn to_string(
        &self,
        user_friendly: bool,
        url_safe: bool,
        bounceable: bool,
        test_only: bool,
    ) -> String {
        if !user_friendly {
            return format!("{}:{}", self.workchain, hex::encode(self.hash_part));
        }

        let mut tag = if bounceable { 0x11u8 } else { 0x51u8 };
        if test_only {
            tag |= 0x80;
        }

        let mut data = Vec::with_capacity(36);
        data.push(tag);
        data.push(self.workchain as u8);
        data.extend_from_slice(&self.hash_part);

        let crc = crc16(&data);
        data.extend_from_slice(&crc);

        if url_safe {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&data)
        } else {
            base64::engine::general_purpose::STANDARD.encode(&data)
        }
    }

    /// Converts to hex format (workchain:hash)
    pub fn to_hex(&self) -> String {
        format!("{}:{}", self.workchain, hex::encode(self.hash_part))
    }

    /// Converts to user-friendly base64 format
    pub fn to_base64(&self) -> String {
        self.to_string(true, true, self.is_bounceable, self.is_test_only)
    }

    /// Sets the bounceable flag
    pub fn set_bounceable(&mut self, bounceable: bool) {
        self.is_bounceable = bounceable;
    }

    /// Sets the test-only flag
    pub fn set_test_only(&mut self, test_only: bool) {
        self.is_test_only = test_only;
    }

    /// Converts to TL AccountId
    pub fn to_account_id(&self) -> AccountId {
        AccountId {
            workchain: self.workchain as i32,
            id: Int256(self.hash_part.clone()),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base64())
    }
}

impl std::str::FromStr for Address {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Address::from_str(s)
    }
}

/// Represents an external address
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAddress {
    /// The external address value
    pub value: Option<u64>,
    /// Bit length of the address
    pub bit_len: usize,
}

impl ExternalAddress {
    /// Creates a new external address
    pub fn new(value: Option<u64>, bit_len: Option<usize>) -> Self {
        let bit_len =
            bit_len.unwrap_or_else(|| value.map(|v| 64 - v.leading_zeros() as usize).unwrap_or(0));

        Self { value, bit_len }
    }

    /// Creates an external address from bytes
    pub fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::new(None, Some(0));
        }

        let mut value = 0u64;
        for (i, &byte) in bytes.iter().take(8).enumerate() {
            value |= (byte as u64) << ((bytes.len() - 1 - i) * 8);
        }

        Self::new(Some(value), Some(bytes.len() * 8))
    }

    /// Creates an external address from hex string
    pub fn from_hex(hex: &str) -> Result<Self> {
        let bytes = hex::decode(hex)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl fmt::Display for ExternalAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value {
            Some(v) if self.bit_len > 0 => write!(f, "ExternalAddress<{:#x}>", v),
            _ => write!(f, "ExternalAddress<null>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_hex() {
        let addr =
            Address::from_hex("0:83dfd552e63729b472fcbcc8c45ebcc6691702558b68ec7527e1ba403a0f31a8")
                .unwrap();
        assert_eq!(addr.workchain, 0);
        assert_eq!(
            addr.to_hex(),
            "0:83dfd552e63729b472fcbcc8c45ebcc6691702558b68ec7527e1ba403a0f31a8"
        );
    }

    #[test]
    fn test_address_base64() {
        let addr_str = "EQCD39VS5jcptHL8vMjEXrzGaRcCVYto7HUn4bpAOg8xqB2N";
        let addr = Address::from_base64(addr_str).unwrap();
        assert_eq!(addr.workchain, 0);
        assert!(addr.is_bounceable);
    }

    #[test]
    fn test_address_conversion() {
        let addr = Address::new(
            0,
            [
                0x83, 0xdf, 0xd5, 0x52, 0xe6, 0x37, 0x29, 0xb4, 0x72, 0xfc, 0xbc, 0xc8, 0xc4, 0x5e,
                0xbc, 0xc6, 0x69, 0x17, 0x02, 0x55, 0x8b, 0x68, 0xec, 0x75, 0x27, 0xe1, 0xba, 0x40,
                0x3a, 0x0f, 0x31, 0xa8,
            ],
        );

        let hex = addr.to_hex();
        let parsed = Address::from_hex(&hex).unwrap();
        assert_eq!(addr, parsed);
    }

    #[test]
    fn test_external_address() {
        let ext = ExternalAddress::new(Some(0x1234), Some(16));
        assert_eq!(ext.value, Some(0x1234));
        assert_eq!(ext.bit_len, 16);
    }

    #[test]
    fn test_zero_address_formats() {
        // Create zero address (0:0000000000000000000000000000000000000000000000000000000000000000)
        let zero_addr = Address::new(0, [0u8; 32]);

        // Print in raw format
        let raw = zero_addr.to_hex();
        println!("Raw format: {}", raw);

        // Print in base64 bounceable format
        let base64_bounceable = zero_addr.to_string(true, true, true, false);
        println!("Base64 bounceable: {}", base64_bounceable);

        // Print in base64 non-bounceable format
        let base64_non_bounceable = zero_addr.to_string(true, true, false, false);
        println!("Base64 non-bounceable: {}", base64_non_bounceable);

        // Verify the formats
        assert_eq!(
            raw,
            "0:0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            base64_bounceable,
            "EQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM9c"
        );
        assert_eq!(
            base64_non_bounceable,
            "UQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJKZ"
        );
    }
}
