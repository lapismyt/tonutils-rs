//! TON Address implementation
//!
//! Supports both internal addresses (workchain + hash) and external addresses.

use crate::tl::{Int256, common::AccountId};
use anyhow::{Result, bail};
use base64::Engine;
use std::fmt;

const BOUNCEABLE_TAG: u8 = 0x11;
const NON_BOUNCEABLE_TAG: u8 = 0x51;
const TEST_ONLY_FLAG: u8 = 0x80;
const USER_FRIENDLY_ADDRESS_LEN: usize = 36;
const RAW_HASH_HEX_LEN: usize = 64;

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

    /// Parses address from raw format: "workchain:hash".
    pub fn from_hex(address: &str) -> Result<Self> {
        let (workchain, hash_hex) = address
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Invalid raw address format: missing separator"))?;

        if hash_hex.contains(':') {
            bail!("Invalid raw address format: too many separators");
        }

        let workchain = workchain
            .parse::<i8>()
            .map_err(|_| anyhow::anyhow!("Invalid address workchain"))?;
        validate_workchain(workchain)?;

        if hash_hex.len() != RAW_HASH_HEX_LEN {
            bail!(
                "Invalid address hash length: expected {} hex characters",
                RAW_HASH_HEX_LEN
            );
        }

        let hash_bytes =
            hex::decode(hash_hex).map_err(|_| anyhow::anyhow!("Invalid address hash hex"))?;
        let mut hash_part = [0u8; 32];
        hash_part.copy_from_slice(&hash_bytes);

        Ok(Self::new(workchain, hash_part))
    }

    /// Parses address from base64 user-friendly format
    pub fn from_base64(address: &str) -> Result<Self> {
        let decoded = decode_user_friendly(address)?;

        if decoded.len() != USER_FRIENDLY_ADDRESS_LEN {
            bail!(
                "Invalid base64 address length: expected {} bytes",
                USER_FRIENDLY_ADDRESS_LEN
            );
        }

        let mut tag = decoded[0];
        let mut is_test_only = false;

        // Check test flag
        if tag & TEST_ONLY_FLAG != 0 {
            is_test_only = true;
            tag ^= TEST_ONLY_FLAG;
        }

        let is_bounceable = match tag {
            BOUNCEABLE_TAG => true,
            NON_BOUNCEABLE_TAG => false,
            _ => bail!("Invalid address tag"),
        };

        let workchain = decoded[1] as i8;
        validate_workchain(workchain)?;
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
            return self.to_raw();
        }

        let mut tag = if bounceable {
            BOUNCEABLE_TAG
        } else {
            NON_BOUNCEABLE_TAG
        };
        if test_only {
            tag |= TEST_ONLY_FLAG;
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

    /// Converts to raw format (workchain:hash).
    pub fn to_raw(&self) -> String {
        format!("{}:{}", self.workchain, hex::encode(self.hash_part))
    }

    /// Converts to hex raw format (workchain:hash).
    pub fn to_hex(&self) -> String {
        self.to_raw()
    }

    /// Converts to user-friendly URL-safe base64 without padding.
    pub fn to_base64(&self) -> String {
        self.to_user_friendly_url_safe()
    }

    /// Converts to user-friendly URL-safe base64 without padding using stored flags.
    pub fn to_user_friendly_url_safe(&self) -> String {
        self.to_string(true, true, self.is_bounceable, self.is_test_only)
    }

    /// Converts to user-friendly standard base64 with padding using stored flags.
    pub fn to_user_friendly_base64(&self) -> String {
        self.to_string(true, false, self.is_bounceable, self.is_test_only)
    }

    /// Converts to user-friendly bounceable address using stored URL-safe preference.
    pub fn to_bounceable(&self, url_safe: bool) -> String {
        self.to_string(true, url_safe, true, self.is_test_only)
    }

    /// Converts to user-friendly non-bounceable address using stored test-only flag.
    pub fn to_non_bounceable(&self, url_safe: bool) -> String {
        self.to_string(true, url_safe, false, self.is_test_only)
    }

    /// Converts to user-friendly test-only address using stored bounceable flag.
    pub fn to_test_only(&self, url_safe: bool) -> String {
        self.to_string(true, url_safe, self.is_bounceable, true)
    }

    /// Converts to user-friendly non-test-only address using stored bounceable flag.
    pub fn to_non_test_only(&self, url_safe: bool) -> String {
        self.to_string(true, url_safe, self.is_bounceable, false)
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

fn validate_workchain(workchain: i8) -> Result<()> {
    if workchain < -1 {
        bail!("Invalid address workchain: {}", workchain);
    }
    Ok(())
}

fn decode_user_friendly(address: &str) -> Result<Vec<u8>> {
    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(address) {
        return Ok(decoded);
    }
    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE.decode(address) {
        return Ok(decoded);
    }
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD_NO_PAD.decode(address) {
        return Ok(decoded);
    }
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(address) {
        return Ok(decoded);
    }

    bail!("Invalid base64 address encoding")
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

    struct AddressFixture {
        name: &'static str,
        raw: &'static str,
        user_friendly_url_safe: &'static str,
        user_friendly_standard: &'static str,
        bounceable: bool,
        test_only: bool,
    }

    const TON_DOCS_ADDRESS_FIXTURES: &[AddressFixture] = &[
        AddressFixture {
            name: "ton-docs-bounceable-mainnet",
            raw: "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e",
            user_friendly_url_safe: "EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHF",
            user_friendly_standard: "EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff+W72r5gqPrHF",
            bounceable: true,
            test_only: false,
        },
        AddressFixture {
            name: "ton-docs-non-bounceable-mainnet",
            raw: "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e",
            user_friendly_url_safe: "UQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPuwA",
            user_friendly_standard: "UQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff+W72r5gqPuwA",
            bounceable: false,
            test_only: false,
        },
        AddressFixture {
            name: "ton-docs-bounceable-testnet",
            raw: "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e",
            user_friendly_url_safe: "kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP",
            user_friendly_standard: "kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff+W72r5gqPgpP",
            bounceable: true,
            test_only: true,
        },
        AddressFixture {
            name: "ton-docs-non-bounceable-testnet",
            raw: "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e",
            user_friendly_url_safe: "0QDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPleK",
            user_friendly_standard: "0QDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff+W72r5gqPleK",
            bounceable: false,
            test_only: true,
        },
    ];

    #[test]
    fn test_ton_docs_user_friendly_address_fixtures() {
        for fixture in TON_DOCS_ADDRESS_FIXTURES {
            for encoded in [
                fixture.user_friendly_url_safe,
                fixture.user_friendly_standard,
            ] {
                let parsed = Address::from_base64(encoded).unwrap_or_else(|err| {
                    panic!("{} failed to parse {}: {}", fixture.name, encoded, err)
                });
                assert_eq!(parsed.to_raw(), fixture.raw, "{}", fixture.name);
                assert_eq!(parsed.is_bounceable, fixture.bounceable, "{}", fixture.name);
                assert_eq!(parsed.is_test_only, fixture.test_only, "{}", fixture.name);
                assert_eq!(
                    parsed.to_user_friendly_url_safe(),
                    fixture.user_friendly_url_safe,
                    "{}",
                    fixture.name
                );
                assert_eq!(
                    parsed.to_user_friendly_base64(),
                    fixture.user_friendly_standard,
                    "{}",
                    fixture.name
                );
            }
        }
    }

    #[test]
    fn test_zero_address_user_friendly_fixture_variants() {
        let raw = "0:0000000000000000000000000000000000000000000000000000000000000000";
        let bounceable =
            Address::from_base64("EQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM9c").unwrap();
        let non_bounceable =
            Address::from_base64("UQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJKZ").unwrap();

        assert_eq!(bounceable.to_raw(), raw);
        assert!(bounceable.is_bounceable);
        assert!(!bounceable.is_test_only);
        assert_eq!(non_bounceable.to_raw(), raw);
        assert!(!non_bounceable.is_bounceable);
        assert!(!non_bounceable.is_test_only);
    }

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
    fn test_address_user_friendly_variants_roundtrip() {
        let mut addr = Address::new(0, [0xAB; 32]);
        addr.set_bounceable(false);
        addr.set_test_only(true);

        let url_safe = addr.to_user_friendly_url_safe();
        let standard = addr.to_user_friendly_base64();
        let standard_no_pad = standard.trim_end_matches('=');

        for encoded in [url_safe.as_str(), standard.as_str(), standard_no_pad] {
            let parsed = Address::from_base64(encoded).unwrap();
            assert_eq!(parsed.workchain, addr.workchain);
            assert_eq!(parsed.hash_part, addr.hash_part);
            assert!(!parsed.is_bounceable);
            assert!(parsed.is_test_only);
        }
    }

    #[test]
    fn test_address_explicit_format_helpers() {
        let addr = Address::new(-1, [0x22; 32]);

        assert_eq!(
            addr.to_raw(),
            "-1:2222222222222222222222222222222222222222222222222222222222222222"
        );
        assert_eq!(addr.to_hex(), addr.to_raw());

        let bounceable = Address::from_base64(&addr.to_bounceable(true)).unwrap();
        assert!(bounceable.is_bounceable);

        let non_bounceable = Address::from_base64(&addr.to_non_bounceable(false)).unwrap();
        assert!(!non_bounceable.is_bounceable);

        let test_only = Address::from_base64(&addr.to_test_only(true)).unwrap();
        assert!(test_only.is_test_only);

        let non_test_only = Address::from_base64(&test_only.to_non_test_only(true)).unwrap();
        assert!(!non_test_only.is_test_only);
    }

    #[test]
    fn test_address_rejects_invalid_raw_inputs() {
        assert!(
            Address::from_hex("0:1234")
                .unwrap_err()
                .to_string()
                .contains("hash length")
        );
        assert!(
            Address::from_hex(
                "-2:0000000000000000000000000000000000000000000000000000000000000000"
            )
            .unwrap_err()
            .to_string()
            .contains("workchain")
        );
        assert!(
            Address::from_hex("0:gg00000000000000000000000000000000000000000000000000000000000000")
                .unwrap_err()
                .to_string()
                .contains("hash hex")
        );
    }

    #[test]
    fn test_address_rejects_invalid_user_friendly_inputs() {
        let addr = Address::new(0, [0x11; 32]);
        let mut decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(addr.to_base64())
            .unwrap();

        decoded[0] = 0x00;
        let invalid_tag = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded);
        assert!(
            Address::from_base64(&invalid_tag)
                .unwrap_err()
                .to_string()
                .contains("tag")
        );

        decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(addr.to_base64())
            .unwrap();
        decoded[10] ^= 1;
        let invalid_crc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded);
        assert!(
            Address::from_base64(&invalid_crc)
                .unwrap_err()
                .to_string()
                .contains("CRC")
        );

        decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(addr.to_base64())
            .unwrap();
        decoded[1] = 0xFE;
        let crc = crc16(&decoded[..34]);
        decoded[34..36].copy_from_slice(&crc);
        let invalid_workchain = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&decoded);
        assert!(
            Address::from_base64(&invalid_workchain)
                .unwrap_err()
                .to_string()
                .contains("workchain")
        );

        assert!(
            Address::from_base64("abcd")
                .unwrap_err()
                .to_string()
                .contains("length")
        );
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
