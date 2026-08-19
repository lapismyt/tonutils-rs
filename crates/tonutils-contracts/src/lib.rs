//! Low-level smart-contract provider traits and raw message helpers.

#[path = "lib_impl.rs"]
pub mod contracts;

pub use contracts::*;

/// LiteAPI `runSmcMethod` mode bit that requests the result stack BoC.
pub const RUN_METHOD_MODE_RETURN_RESULT: u32 = 1 << 2;

/// Converts a get-method name to the conventional TVM method identifier.
pub fn method_name_to_id(name: &str) -> u64 {
    let method_value = tonutils_crc::CRC16.checksum(name.as_bytes()) as u32;
    ((method_value & 0xFFFF) | 0x10000) as u64
}
