//! Tests for TL (Type Language) module

use crate::tl::common::*;
use std::str::FromStr;

#[test]
fn test_int256_creation_and_display() {
    let int256 = Int256([1u8; 32]);
    let hex_str = int256.to_hex();
    assert_eq!(hex_str.len(), 64);
    
    // Test from_hex
    let parsed = Int256::from_hex(&hex_str).unwrap();
    assert_eq!(parsed, int256);
}

#[test]
fn test_int256_random() {
    let int1 = Int256::random();
    let int2 = Int256::random();
    // Random values should be different (with extremely high probability)
    assert_ne!(int1, int2);
}

#[test]
fn test_int256_from_str() {
    let hex_str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let int256 = Int256::from_str(hex_str).unwrap();
    assert_eq!(int256.to_hex(), hex_str);
}

#[test]
fn test_int256_default() {
    let default_int = Int256::default();
    assert_eq!(default_int.0, [0u8; 32]);
}

#[test]
fn test_block_id_creation() {
    let block_id = BlockId {
        workchain: -1,
        shard: 0x8000000000000000u64 as i64,
        seqno: 12345,
    };
    
    assert_eq!(block_id.workchain, -1);
    assert_eq!(block_id.shard, 0x8000000000000000u64 as i64);
    assert_eq!(block_id.seqno, 12345);
}

#[test]
fn test_block_id_ext_creation() {
    let root_hash = Int256([1u8; 32]);
    let file_hash = Int256([2u8; 32]);
    
    let block_id_ext = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 1000,
        root_hash: root_hash.clone(),
        file_hash: file_hash.clone(),
    };
    
    assert_eq!(block_id_ext.workchain, 0);
    assert_eq!(block_id_ext.root_hash, root_hash);
    assert_eq!(block_id_ext.file_hash, file_hash);
}

#[test]
fn test_block_id_ext_display() {
    let root_hash = Int256([0xABu8; 32]);
    let file_hash = Int256([0xCDu8; 32]);
    
    let block_id_ext = BlockIdExt {
        workchain: -1,
        shard: 0x8000000000000000u64 as i64,
        seqno: 100,
        root_hash,
        file_hash,
    };
    
    let display_str = format!("{}", block_id_ext);
    assert!(display_str.contains("-1"));
    assert!(display_str.contains("8000000000000000"));
    assert!(display_str.contains("100"));
}

#[test]
fn test_account_id_creation() {
    let account_id = AccountId {
        workchain: 0,
        id: Int256([0x42u8; 32]),
    };
    
    assert_eq!(account_id.workchain, 0);
    assert_eq!(account_id.id.0[0], 0x42);
}

#[test]
fn test_transaction_id3_creation() {
    let tx_id = TransactionId3 {
        account: Int256([0x11u8; 32]),
        lt: 123456789,
    };
    
    assert_eq!(tx_id.lt, 123456789);
    assert_eq!(tx_id.account.0[0], 0x11);
}

#[test]
fn test_signature_creation() {
    let signature = Signature {
        node_id_short: Int256([0xAAu8; 32]),
        signature: vec![1, 2, 3, 4, 5],
    };
    
    assert_eq!(signature.signature.len(), 5);
    assert_eq!(signature.node_id_short.0[0], 0xAA);
}

#[test]
fn test_signature_set_creation() {
    let sig1 = Signature {
        node_id_short: Int256([0x01u8; 32]),
        signature: vec![1, 2, 3],
    };
    
    let sig2 = Signature {
        node_id_short: Int256([0x02u8; 32]),
        signature: vec![4, 5, 6],
    };
    
    let sig_set = SignatureSet {
        validator_set_hash: 0x12345678,
        catchain_seqno: 42,
        signatures: vec![sig1, sig2],
    };
    
    assert_eq!(sig_set.validator_set_hash, 0x12345678);
    assert_eq!(sig_set.catchain_seqno, 42);
    assert_eq!(sig_set.signatures.len(), 2);
}

#[test]
fn test_zero_state_id_ext_creation() {
    let zero_state = ZeroStateIdExt {
        workchain: -1,
        root_hash: Int256([0x33u8; 32]),
        file_hash: Int256([0x44u8; 32]),
    };
    
    assert_eq!(zero_state.workchain, -1);
    assert_eq!(zero_state.root_hash.0[0], 0x33);
    assert_eq!(zero_state.file_hash.0[0], 0x44);
}

#[test]
fn test_transaction_id_with_all_fields() {
    let tx_id = TransactionId {
        mode: (),
        account: Some(Int256([0x55u8; 32])),
        lt: Some(999),
        hash: Some(Int256([0x66u8; 32])),
    };
    
    assert!(tx_id.account.is_some());
    assert!(tx_id.lt.is_some());
    assert!(tx_id.hash.is_some());
    assert_eq!(tx_id.lt.unwrap(), 999);
}

#[test]
fn test_transaction_id_partial_fields() {
    let tx_id = TransactionId {
        mode: (),
        account: Some(Int256([0x77u8; 32])),
        lt: None,
        hash: None,
    };
    
    assert!(tx_id.account.is_some());
    assert!(tx_id.lt.is_none());
    assert!(tx_id.hash.is_none());
}

#[test]
fn test_library_entry_creation() {
    let entry = LibraryEntry {
        hash: Int256([0x88u8; 32]),
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
    };
    
    assert_eq!(entry.hash.0[0], 0x88);
    assert_eq!(entry.data.len(), 8);
}

#[test]
fn test_string_creation_from_str() {
    let tl_string = String::from("Hello, World!");
    assert_eq!(format!("{}", tl_string), "Hello, World!");
}

#[test]
fn test_string_creation_from_string() {
    let rust_string = std::string::String::from("Test String");
    let tl_string = String::new(rust_string);
    assert_eq!(format!("{}", tl_string), "Test String");
}

#[test]
fn test_block_link_back_creation() {
    let from = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 100,
        root_hash: Int256([0x11u8; 32]),
        file_hash: Int256([0x22u8; 32]),
    };
    
    let to = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 101,
        root_hash: Int256([0x33u8; 32]),
        file_hash: Int256([0x44u8; 32]),
    };
    
    let link = BlockLink::BlockLinkBack {
        to_key_block: true,
        from: from.clone(),
        to: to.clone(),
        dest_proof: vec![1, 2, 3],
        proof: vec![4, 5, 6],
        state_proof: vec![7, 8, 9],
    };
    
    match link {
        BlockLink::BlockLinkBack { to_key_block, from: f, to: t, .. } => {
            assert!(to_key_block);
            assert_eq!(f.seqno, 100);
            assert_eq!(t.seqno, 101);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_block_link_forward_creation() {
    let from = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 200,
        root_hash: Int256([0x55u8; 32]),
        file_hash: Int256([0x66u8; 32]),
    };
    
    let to = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 201,
        root_hash: Int256([0x77u8; 32]),
        file_hash: Int256([0x88u8; 32]),
    };
    
    let sig_set = SignatureSet {
        validator_set_hash: 0xABCDEF,
        catchain_seqno: 5,
        signatures: vec![],
    };
    
    let link = BlockLink::BlockLinkForward {
        to_key_block: false,
        from: from.clone(),
        to: to.clone(),
        dest_proof: vec![10, 11],
        config_proof: vec![12, 13],
        signatures: sig_set,
    };
    
    match link {
        BlockLink::BlockLinkForward { to_key_block, signatures, .. } => {
            assert!(!to_key_block);
            assert_eq!(signatures.catchain_seqno, 5);
        }
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_int256_equality() {
    let int1 = Int256([0xABu8; 32]);
    let int2 = Int256([0xABu8; 32]);
    let int3 = Int256([0xCDu8; 32]);
    
    assert_eq!(int1, int2);
    assert_ne!(int1, int3);
}

#[test]
fn test_block_id_ext_equality() {
    let block1 = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 100,
        root_hash: Int256([0x11u8; 32]),
        file_hash: Int256([0x22u8; 32]),
    };
    
    let block2 = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 100,
        root_hash: Int256([0x11u8; 32]),
        file_hash: Int256([0x22u8; 32]),
    };
    
    let block3 = BlockIdExt {
        workchain: 0,
        shard: 0x8000000000000000u64 as i64,
        seqno: 101,
        root_hash: Int256([0x11u8; 32]),
        file_hash: Int256([0x22u8; 32]),
    };
    
    assert_eq!(block1, block2);
    assert_ne!(block1, block3);
}

#[test]
fn test_int256_hash_consistency() {
    use std::collections::HashMap;
    
    let int1 = Int256([0x42u8; 32]);
    let int2 = Int256([0x42u8; 32]);
    
    let mut map = HashMap::new();
    map.insert(int1.clone(), "value1");
    map.insert(int2, "value2"); // Should replace value1
    
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&int1), Some(&"value2"));
}
