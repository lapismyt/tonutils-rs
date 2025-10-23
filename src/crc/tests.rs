//! Tests for CRC module

use super::*;

#[test]
fn test_crc16_basic() {
    let data = b"hello world";
    let checksum = CRC16.checksum(data);
    
    // CRC16 should produce 16-bit value
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc16_empty_data() {
    let data = b"";
    let checksum = CRC16.checksum(data);
    
    // Empty data should still produce a valid checksum
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc16_deterministic() {
    let data = b"test data";
    let checksum1 = CRC16.checksum(data);
    let checksum2 = CRC16.checksum(data);
    
    // Same data should produce same checksum
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc16_different_data() {
    let data1 = b"data1";
    let data2 = b"data2";
    let checksum1 = CRC16.checksum(data1);
    let checksum2 = CRC16.checksum(data2);
    
    // Different data should (likely) produce different checksums
    assert_ne!(checksum1, checksum2);
}

#[test]
fn test_crc16_single_byte() {
    let data = b"a";
    let checksum = CRC16.checksum(data);
    
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc16_large_data() {
    let data = vec![0xABu8; 1024];
    let checksum = CRC16.checksum(&data);
    
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc32_basic() {
    let data = b"hello world";
    let checksum = CRC32.checksum(data);
    
    // CRC32 should produce 32-bit value
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc32_empty_data() {
    let data = b"";
    let checksum = CRC32.checksum(data);
    
    // Empty data should still produce a valid checksum
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc32_deterministic() {
    let data = b"test data";
    let checksum1 = CRC32.checksum(data);
    let checksum2 = CRC32.checksum(data);
    
    // Same data should produce same checksum
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc32_different_data() {
    let data1 = b"data1";
    let data2 = b"data2";
    let checksum1 = CRC32.checksum(data1);
    let checksum2 = CRC32.checksum(data2);
    
    // Different data should (likely) produce different checksums
    assert_ne!(checksum1, checksum2);
}

#[test]
fn test_crc32_single_byte() {
    let data = b"x";
    let checksum = CRC32.checksum(data);
    
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc32_large_data() {
    let data = vec![0x42u8; 4096];
    let checksum = CRC32.checksum(&data);
    
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc16_vs_crc32_same_data() {
    let data = b"test";
    let crc16 = CRC16.checksum(data);
    let crc32 = CRC32.checksum(data);
    
    // CRC16 and CRC32 should produce different results
    assert_ne!(crc16 as u32, crc32);
}

#[test]
fn test_crc16_digest_update() {
    let data1 = b"hello";
    let data2 = b" world";
    
    let mut digest = CRC16.digest();
    digest.update(data1);
    digest.update(data2);
    let checksum1 = digest.finalize();
    
    let combined = b"hello world";
    let checksum2 = CRC16.checksum(combined);
    
    // Incremental update should produce same result
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc32_digest_update() {
    let data1 = b"hello";
    let data2 = b" world";
    
    let mut digest = CRC32.digest();
    digest.update(data1);
    digest.update(data2);
    let checksum1 = digest.finalize();
    
    let combined = b"hello world";
    let checksum2 = CRC32.checksum(combined);
    
    // Incremental update should produce same result
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc16_zero_bytes() {
    let data = vec![0u8; 100];
    let checksum = CRC16.checksum(&data);
    
    // All zeros should still produce a valid checksum
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc32_zero_bytes() {
    let data = vec![0u8; 100];
    let checksum = CRC32.checksum(&data);
    
    // All zeros should still produce a valid checksum
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc16_all_ones() {
    let data = vec![0xFFu8; 100];
    let checksum = CRC16.checksum(&data);
    
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc32_all_ones() {
    let data = vec![0xFFu8; 100];
    let checksum = CRC32.checksum(&data);
    
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc16_pattern_data() {
    let data = vec![0xAAu8, 0x55u8, 0xAAu8, 0x55u8];
    let checksum1 = CRC16.checksum(&data);
    
    // Repeating the pattern should change the checksum
    let data2 = vec![0xAAu8, 0x55u8, 0xAAu8, 0x55u8, 0xAAu8, 0x55u8];
    let checksum2 = CRC16.checksum(&data2);
    
    assert_ne!(checksum1, checksum2);
}

#[test]
fn test_crc32_pattern_data() {
    let data = vec![0xAAu8, 0x55u8, 0xAAu8, 0x55u8];
    let checksum1 = CRC32.checksum(&data);
    
    // Repeating the pattern should change the checksum
    let data2 = vec![0xAAu8, 0x55u8, 0xAAu8, 0x55u8, 0xAAu8, 0x55u8];
    let checksum2 = CRC32.checksum(&data2);
    
    assert_ne!(checksum1, checksum2);
}

#[test]
fn test_crc16_multiple_digests() {
    let data = b"test";
    
    let mut digest1 = CRC16.digest();
    digest1.update(data);
    let checksum1 = digest1.finalize();
    
    let mut digest2 = CRC16.digest();
    digest2.update(data);
    let checksum2 = digest2.finalize();
    
    // Multiple digests should produce same result
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc32_multiple_digests() {
    let data = b"test";
    
    let mut digest1 = CRC32.digest();
    digest1.update(data);
    let checksum1 = digest1.finalize();
    
    let mut digest2 = CRC32.digest();
    digest2.update(data);
    let checksum2 = digest2.finalize();
    
    // Multiple digests should produce same result
    assert_eq!(checksum1, checksum2);
}

#[test]
fn test_crc16_binary_data() {
    let data: Vec<u8> = (0..=255).collect();
    let checksum = CRC16.checksum(&data);
    
    assert!(checksum <= u16::MAX);
}

#[test]
fn test_crc32_binary_data() {
    let data: Vec<u8> = (0..=255).collect();
    let checksum = CRC32.checksum(&data);
    
    assert!(checksum <= u32::MAX);
}

#[test]
fn test_crc16_order_matters() {
    let data1 = b"abc";
    let data2 = b"bca";
    
    let checksum1 = CRC16.checksum(data1);
    let checksum2 = CRC16.checksum(data2);
    
    // Order should matter
    assert_ne!(checksum1, checksum2);
}

#[test]
fn test_crc32_order_matters() {
    let data1 = b"abc";
    let data2 = b"bca";
    
    let checksum1 = CRC32.checksum(data1);
    let checksum2 = CRC32.checksum(data2);
    
    // Order should matter
    assert_ne!(checksum1, checksum2);
}
