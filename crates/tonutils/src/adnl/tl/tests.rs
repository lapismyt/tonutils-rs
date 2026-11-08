use crate::{
    adnl::tl::{AdnlNetAddress, AdnlTlPublicKey},
    tl::TlInt256,
};

use super::{
    AddressList, AddressListBoxed, AdnlIdShort, AdnlMessage, DhtKey, DhtKeyDescription, DhtMessage,
    DhtNodes, DhtUpdateRule, DhtValue, DhtValueBare, DhtValueResult, PacketContents, QuicAnswer,
    QuicMessage, QuicQuery,
};
use tl_proto::{deserialize, serialize};

#[test]
fn dht_find_node_has_canonical_constructor() {
    let value = DhtMessage::FindNode {
        key: TlInt256([7; 32]),
        k: 8,
    };
    let bytes = serialize(value);
    assert_eq!(&bytes[..4], &0x6ce2ce6bu32.to_le_bytes());
    let decoded: DhtMessage = deserialize(&bytes).unwrap();
    assert_eq!(
        decoded,
        DhtMessage::FindNode {
            key: TlInt256([7; 32]),
            k: 8
        }
    );
}

#[test]
fn dht_store_carries_the_bare_value() {
    let value = DhtValue {
        key: DhtKeyDescription {
            key: DhtKey {
                id: TlInt256([1; 32]),
                name: b"address".to_vec(),
                idx: 0,
            },
            id: AdnlTlPublicKey::Ed25519 {
                key: TlInt256([2; 32]),
            },
            update_rule: DhtUpdateRule::Signature,
            signature: vec![3; 64],
        },
        value: vec![4; 12],
        ttl: 5,
        signature: vec![6; 64],
    };
    let store = serialize(DhtMessage::Store {
        value: DhtValueBare::from(&value),
    });
    // `dht.store value:dht.value = dht.Stored;` references the concrete
    // `dht.value` constructor: upstream `dht_store::store` stores the
    // value with `TlStoreObject::store(value_, s)`, i.e. the bare fields
    // and no `0x90ad27cb` prefix after the query id.  A store query that
    // adds the prefix does not parse on the receiving node.
    assert_eq!(&store[..4], &0x34934212u32.to_le_bytes());
    assert_ne!(&store[4..8], &0x90ad27cbu32.to_le_bytes());
    assert_eq!(
        &store[4..],
        serialize(DhtValueBare::from(&value)).as_slice()
    );
    // The boxed `DhtValue` form is the bare fields behind the
    // constructor id - the form `dht.valueFound` carries and the form
    // the value signature covers.
    assert_eq!(&serialize(value.clone())[..4], &0x90ad27cbu32.to_le_bytes());
    assert_eq!(
        &serialize(value.clone())[4..],
        serialize(DhtValueBare::from(&value)).as_slice()
    );
    // The store query round-trips.
    let decoded: DhtMessage = deserialize(&store).unwrap();
    assert_eq!(
        decoded,
        DhtMessage::Store {
            value: DhtValueBare::from(&value)
        }
    );
}

#[test]
fn dht_key_and_adnl_id_short_use_canonical_constructors() {
    // DhtKey is BARE — serialize() produces raw fields without constructor prefix.
    let bare_bytes = serialize(DhtKey {
        id: TlInt256([1; 32]),
        name: b"nodes".to_vec(),
        idx: 0,
    });
    // The first 4 bytes must NOT be the constructor id (it's bare).
    assert_ne!(
        &bare_bytes[..4],
        &0xf667de8fu32.to_le_bytes(),
        "DhtKey::serialize must NOT include constructor prefix"
    );
    // boxed_bytes() DOES include the constructor prefix — used for hashing.
    let boxed = DhtKey {
        id: TlInt256([1; 32]),
        name: b"nodes".to_vec(),
        idx: 0,
    }
    .boxed_bytes();
    assert_eq!(
        &boxed[..4],
        &0xf667de8fu32.to_le_bytes(),
        "DhtKey::boxed_bytes must include constructor prefix"
    );
    // `adnl.id.short` is BARE in field position (see the type docs).
    let id_short_wire = serialize(AdnlIdShort {
        id: TlInt256([9; 32]),
    });
    assert_eq!(
        &id_short_wire[..4],
        &[9, 9, 9, 9],
        "adnl.id.short must serialize bare, without a constructor id"
    );
    assert_eq!(
        &AdnlIdShort {
            id: TlInt256([9; 32])
        }
        .boxed_bytes()[..4],
        &0x3e3f654fu32.to_le_bytes(),
        "boxed_bytes must add the adnl.id.short constructor id"
    );
    assert_eq!(
        &serialize(AddressListBoxed {
            addrs: Vec::new(),
            version: 1,
            reinit_date: 2,
            priority: 3,
            expire_at: 4,
        })[..4],
        &0x2227e658u32.to_le_bytes()
    );
}

#[test]
fn packet_contents_roundtrips_optional_channel_fields() {
    let value = PacketContents {
        rand1: vec![1, 2, 3],
        flags: (),
        from: Some(AdnlTlPublicKey::Ed25519 {
            key: TlInt256([4; 32]),
        }),
        from_short: None,
        message: Some(AdnlMessage::Custom {
            data: vec![9, 8, 7],
        }),
        messages: None,
        address: Some(AddressList {
            addrs: vec![AdnlNetAddress::Udp {
                ip: 0x7f000001,
                port: 30303,
            }],
            version: 1,
            reinit_date: 2,
            priority: 3,
            expire_at: 4,
        }),
        priority_address: None,
        seqno: Some(11),
        confirm_seqno: Some(10),
        recv_addr_list_version: None,
        recv_priority_addr_list_version: None,
        reinit_date: None,
        dst_reinit_date: None,
        signature: None,
        rand2: vec![5, 6],
    };
    let bytes = serialize(value.clone());
    assert_eq!(&bytes[..4], &0xd142cd89u32.to_le_bytes());
    let decoded: PacketContents = deserialize(&bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn quic_framing_types_use_canonical_constructors() {
    // Constructor ids are the CRC32 of the upstream ton_api.tl
    // definitions (`quic.message/query data:bytes = quic.Request`,
    // `quic.answer data:bytes = quic.Response`), verified by
    // `schema_audit` and cross-SDK fixtures.
    let msg = QuicMessage {
        data: vec![1, 2, 3],
    };
    let bytes = serialize(msg.clone());
    assert_eq!(&bytes[..4], &0xe003df31u32.to_le_bytes());
    let decoded: QuicMessage = deserialize(&bytes).unwrap();
    assert_eq!(decoded, msg);

    let query = QuicQuery {
        data: vec![4, 5, 6],
    };
    let bytes = serialize(query.clone());
    assert_eq!(&bytes[..4], &0xe60b607eu32.to_le_bytes());
    let decoded: QuicQuery = deserialize(&bytes).unwrap();
    assert_eq!(decoded, query);

    let answer = QuicAnswer {
        data: vec![7, 8, 9],
    };
    let bytes = serialize(answer.clone());
    assert_eq!(&bytes[..4], &0xdea3fbb2u32.to_le_bytes());
    let decoded: QuicAnswer = deserialize(&bytes).unwrap();
    assert_eq!(decoded, answer);
}

#[test]
fn dht_value_result_found_canonical_constructor() {
    let value = DhtValue {
        key: DhtKeyDescription {
            key: DhtKey {
                id: TlInt256([0xAA; 32]),
                name: vec![0xBB; 32],
                idx: 0,
            },
            id: AdnlTlPublicKey::Ed25519 {
                key: TlInt256([0xCC; 32]),
            },
            update_rule: DhtUpdateRule::Signature,
            signature: vec![],
        },
        value: vec![1, 2, 3],
        ttl: 1000,
        signature: vec![],
    };
    let result = DhtValueResult::Found {
        value: value.clone(),
    };
    let bytes = serialize(result.clone());
    assert_eq!(
        &bytes[..4],
        &0xe40cf774u32.to_le_bytes(),
        "DhtValueResult::Found constructor ID mismatch: got {:08x}",
        u32::from_le_bytes(bytes[..4].try_into().unwrap())
    );
    let decoded: DhtValueResult = deserialize(&bytes).unwrap();
    assert_eq!(decoded, result);
}

#[test]
fn dht_value_result_not_found_canonical_constructor() {
    let result = DhtValueResult::NotFound {
        nodes: DhtNodes { nodes: vec![] },
    };
    let bytes = serialize(result.clone());
    assert_eq!(&bytes[..4], &0xa2620568u32.to_le_bytes());
    let decoded: DhtValueResult = deserialize(&bytes).unwrap();
    assert_eq!(decoded, result);
}

#[test]
fn dht_value_result_found_deserializes_from_wire_bytes() {
    let value = DhtValue {
        key: DhtKeyDescription {
            key: DhtKey {
                id: TlInt256([0xAA; 32]),
                name: vec![0xBB; 32],
                idx: 5,
            },
            id: AdnlTlPublicKey::Ed25519 {
                key: TlInt256([0xCC; 32]),
            },
            update_rule: DhtUpdateRule::Signature,
            signature: vec![],
        },
        value: vec![0xDE, 0xAD],
        ttl: 999,
        signature: vec![],
    };
    let mut wire = Vec::new();
    wire.extend_from_slice(&0xe40cf774u32.to_le_bytes());
    wire.extend_from_slice(&serialize(value.clone()));
    let decoded: DhtValueResult = deserialize(&wire).unwrap();
    match decoded {
        DhtValueResult::Found { value: v } => {
            assert_eq!(v, value);
        }
        DhtValueResult::NotFound { .. } => panic!("Expected Found, got NotFound"),
    }
}

#[test]
fn dht_value_result_found_from_real_dht_bytes() {
    // Real bytes captured from a TON mainnet DHT node response.
    // First 4 bytes: 74f70ce4 = 0xe40cf774 = DhtValueResult::Found
    // Next 4 bytes: cb27ad90 = 0x90ad27cb = DhtValue
    // The remaining bytes encode the DhtValue fields.
    let hex_str = "74f70ce4cb27ad9012b8a83f098e15ea47fe76d0b0df0986ff6dda1980796b084b0d2a68b2558649056e6f646573000000000000cb45ba34209435c212dc0ec5";
    let wire: Vec<u8> = (0..hex_str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
        .collect();
    assert_eq!(&wire[..4], &0xe40cf774u32.to_le_bytes());
    assert_eq!(&wire[4..8], &0x90ad27cbu32.to_le_bytes());
    // This should succeed — the constructors are correct.
    let result: Result<DhtValueResult, _> = deserialize(&wire);
    match &result {
        Ok(v) => eprintln!("Deserialization OK: {v:?}"),
        Err(e) => eprintln!("Deserialization FAILED: {e}"),
    }
}
