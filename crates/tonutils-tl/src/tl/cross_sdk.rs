//! Cross-SDK byte comparison for DHT, overlay, ADNL, and QUIC wire values.
//!
//! `fixtures/cross_sdk/*.json` records canonical logical fields plus the
//! reference `raw_hex` bytes produced by tonutils-go (every case) and
//! pytoniq-core (the cases its pinned schema covers). This test rebuilds the
//! same inputs with tonutils-rs types and asserts that:
//!
//! 1. our serialization is byte-identical to the reference bytes, and
//! 2. the reference bytes roundtrip through our decoders byte-for-byte.
//!
//! Roundtrip alone would not catch field-layout divergence, because parsing
//! and serializing with the same (possibly wrong) layout is self-inverse;
//! byte equality against independently produced reference bytes is the
//! property that matters. Reference bytes are regenerated and verified by
//! `scripts/cross_sdk/gen_go` and `scripts/cross_sdk/gen_py.py`, which CI
//! runs alongside this test. Cases without `fields` are live captures (raw
//! mainnet bytes recorded by the ignored
//! `tonutils-mempool::live::capture_cross_sdk_dht_answer_from_mainnet`
//! test): they have no canonical input, so only the roundtrip property is
//! asserted for them. The `note`/`references` fields mark cases a reference
//! cannot cover because its pinned schema lacks the constructor.

use serde_json::Value;
use tl_proto::{TlRead, TlWrite};

use super::adnl;
use super::common::Int256;
use super::network::{
    Address, AddressList, DhtKey, DhtKeyDescription, DhtMessage, DhtNode, DhtNodeBoxed, DhtNodes,
    DhtNodesBoxed, DhtUpdateRule, DhtValue, DhtValueResult, OverlayNode, OverlayNodes,
    OverlayQuery, PublicKey, QuicAnswer, QuicMessage, QuicQuery,
};

const FIXTURE_FILES: [(&str, &str); 6] = [
    (
        "dht_queries.json",
        include_str!("../../../../fixtures/cross_sdk/dht_queries.json"),
    ),
    (
        "dht_responses.json",
        include_str!("../../../../fixtures/cross_sdk/dht_responses.json"),
    ),
    (
        "overlay_queries.json",
        include_str!("../../../../fixtures/cross_sdk/overlay_queries.json"),
    ),
    (
        "quic_frames.json",
        include_str!("../../../../fixtures/cross_sdk/quic_frames.json"),
    ),
    (
        "adnl_messages.json",
        include_str!("../../../../fixtures/cross_sdk/adnl_messages.json"),
    ),
    (
        "live_dht_answers.json",
        include_str!("../../../../fixtures/cross_sdk/live_dht_answers.json"),
    ),
];
const MANIFEST: &str = include_str!("../../../../fixtures/cross_sdk/manifest.json");

#[test]
fn cross_sdk_manifest_matches_bundled_fixtures() {
    let manifest: Value =
        serde_json::from_str(MANIFEST).expect("fixtures/cross_sdk/manifest.json must parse");
    let mut listed: Vec<String> = manifest["fixtures"]
        .as_array()
        .expect("manifest.fixtures must be an array")
        .iter()
        .map(|entry| entry["path"].as_str().expect("fixture path").to_owned())
        .collect();
    let mut bundled: Vec<String> = FIXTURE_FILES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    listed.sort();
    bundled.sort();
    assert_eq!(
        listed, bundled,
        "manifest fixture list must match the bundled fixture files"
    );

    for (file, content) in FIXTURE_FILES {
        let fixture: Value = serde_json::from_str(content)
            .unwrap_or_else(|error| panic!("fixtures/cross_sdk/{file} must parse: {error}"));
        for case in fixture["cases"].as_array().expect("fixture cases") {
            let name = case["name"].as_str().expect("case name");
            let raw = case["raw_hex"]
                .as_str()
                .unwrap_or_else(|| panic!("{file}: case {name} has no raw_hex"));
            assert!(
                !raw.is_empty() && raw.len() % 2 == 0,
                "{file}: case {name} raw_hex is empty or odd-length; \
                 run scripts/cross_sdk/gen_go with --write"
            );
            assert!(
                hex::decode(raw).is_ok(),
                "{file}: case {name} raw_hex is not valid hex"
            );
        }
    }
}

#[test]
fn cross_sdk_fixtures_match_reference_serialization() {
    let mut total = 0usize;
    for (file, content) in FIXTURE_FILES {
        let fixture: Value = serde_json::from_str(content)
            .unwrap_or_else(|error| panic!("fixtures/cross_sdk/{file} must parse: {error}"));
        for case in fixture["cases"].as_array().expect("fixture cases") {
            run_case(case);
            total += 1;
        }
    }
    assert!(
        total >= 18,
        "expected at least 18 cross-SDK fixture cases, found {total}"
    );
}

fn run_case(case: &Value) {
    let name = case["name"].as_str().expect("case name");
    let constructor = case["constructor"].as_str().expect("case constructor");
    let expected_hex = case["raw_hex"].as_str().expect("case raw_hex");
    let expected =
        hex::decode(expected_hex).unwrap_or_else(|_| panic!("case {name}: raw_hex must decode"));

    // Live captures carry no canonical fields: reference SDKs cannot
    // reconstruct them, so only our roundtrip is asserted.
    let Some(fields) = case.get("fields") else {
        match constructor {
            "dht.nodes" => roundtrip_only::<DhtNodesBoxed>(name, constructor, &expected),
            "dht.valueFound" | "dht.valueNotFound" => {
                roundtrip_only::<DhtValueResult>(name, constructor, &expected);
            }
            other => panic!("case {name}: unsupported live-capture constructor {other}"),
        }
        return;
    };

    match constructor {
        "dht.findNode" => check(
            name,
            constructor,
            DhtMessage::FindNode {
                key: int256_field(fields, "key"),
                k: i32_field(fields, "k"),
            },
            &expected,
        ),
        "dht.findValue" => check(
            name,
            constructor,
            DhtMessage::FindValue {
                key: int256_field(fields, "key"),
                k: i32_field(fields, "k"),
            },
            &expected,
        ),
        "dht.ping" => check(
            name,
            constructor,
            DhtMessage::Ping {
                random_id: u64_field(fields, "random_id"),
            },
            &expected,
        ),
        "dht.getSignedAddressList" => check(
            name,
            constructor,
            DhtMessage::GetSignedAddressList,
            &expected,
        ),
        "dht.nodes" => check(
            name,
            constructor,
            DhtNodesBoxed {
                nodes: dht_node_list(fields, "nodes"),
            },
            &expected,
        ),
        "dht.node" => {
            let node = dht_node(fields);
            check(
                name,
                constructor,
                DhtNodeBoxed {
                    id: node.id,
                    addr_list: node.addr_list,
                    version: node.version,
                    signature: node.signature,
                },
                &expected,
            );
        }
        "dht.valueFound" => check(
            name,
            constructor,
            DhtValueResult::Found {
                value: dht_value(field(fields, "value")),
            },
            &expected,
        ),
        "dht.valueNotFound" => check(
            name,
            constructor,
            DhtValueResult::NotFound {
                nodes: DhtNodes {
                    nodes: dht_node_list(fields, "nodes"),
                },
            },
            &expected,
        ),
        "overlay.getRandomPeers" => check(
            name,
            constructor,
            OverlayQuery::GetRandomPeers {
                peers: OverlayNodes {
                    nodes: arr_field(fields, "peers")
                        .iter()
                        .map(overlay_node)
                        .collect(),
                },
            },
            &expected,
        ),
        "overlay.ping" => check(name, constructor, OverlayQuery::Ping, &expected),
        "overlay.query" => check(
            name,
            constructor,
            OverlayQuery::Query {
                overlay: int256_field(fields, "overlay"),
            },
            &expected,
        ),
        "quic.query" => check(
            name,
            constructor,
            QuicQuery {
                data: bytes_field(fields, "data"),
            },
            &expected,
        ),
        "quic.answer" => check(
            name,
            constructor,
            QuicAnswer {
                data: bytes_field(fields, "data"),
            },
            &expected,
        ),
        "quic.message" => check(
            name,
            constructor,
            QuicMessage {
                data: bytes_field(fields, "data"),
            },
            &expected,
        ),
        "adnl.message.query" => check(
            name,
            constructor,
            adnl::Message::Query {
                query_id: int256_field(fields, "query_id"),
                query: bytes_field(fields, "query"),
            },
            &expected,
        ),
        "adnl.message.answer" => check(
            name,
            constructor,
            adnl::Message::Answer {
                query_id: int256_field(fields, "query_id"),
                answer: bytes_field(fields, "answer"),
            },
            &expected,
        ),
        "adnl.message.nop" => check(name, constructor, adnl::Message::Nop, &expected),
        other => panic!("case {name}: unsupported constructor {other}"),
    }
}

/// Serializes `value` and asserts byte equality with `expected`, then
/// asserts that `expected` roundtrips through the same type unchanged.
fn check<T>(name: &str, constructor: &str, value: T, expected: &[u8])
where
    T: for<'a> TlRead<'a> + TlWrite,
{
    let serialized = tl_proto::serialize(value);
    assert_eq!(
        hex::encode(&serialized),
        hex::encode(expected),
        "case {name} ({constructor}): tonutils-rs bytes diverge from reference"
    );
    roundtrip_only::<T>(name, constructor, expected);
}

/// Asserts that `expected` decodes and re-encodes byte-for-byte through `T`.
fn roundtrip_only<T>(name: &str, constructor: &str, expected: &[u8])
where
    T: for<'a> TlRead<'a> + TlWrite,
{
    let decoded: T = match tl_proto::deserialize(expected) {
        Ok(decoded) => decoded,
        Err(error) => {
            panic!("case {name} ({constructor}): reference bytes failed to decode: {error:?}")
        }
    };
    let reencoded = tl_proto::serialize(decoded);
    assert_eq!(
        hex::encode(&reencoded),
        hex::encode(expected),
        "case {name} ({constructor}): reference bytes do not roundtrip through tonutils-rs"
    );
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("missing field {key} in {value}"))
}

fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key)
        .as_str()
        .unwrap_or_else(|| panic!("field {key} must be a string in {value}"))
}

fn arr_field<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    field(value, key)
        .as_array()
        .unwrap_or_else(|| panic!("field {key} must be an array in {value}"))
}

fn bytes_field(value: &Value, key: &str) -> Vec<u8> {
    hex::decode(str_field(value, key)).unwrap_or_else(|_| panic!("field {key} must be hex"))
}

fn int256_field(value: &Value, key: &str) -> Int256 {
    Int256(
        bytes_field(value, key)
            .try_into()
            .unwrap_or_else(|_| panic!("field {key} must be 32 bytes")),
    )
}

fn i32_field(value: &Value, key: &str) -> i32 {
    let number = field(value, key).as_i64().expect("integer field");
    i32::try_from(number).unwrap_or_else(|_| panic!("field {key} must fit in i32"))
}

fn u64_field(value: &Value, key: &str) -> u64 {
    u64::try_from(field(value, key).as_i64().expect("integer field"))
        .unwrap_or_else(|_| panic!("field {key} must fit in u64"))
}

/// Dotted IPv4 to the wire int: the global-config style big-endian value
/// (`127.0.0.1` = `0x7f000001`), which `tl-proto` writes as little-endian.
fn ip_field(value: &Value) -> i32 {
    let octets: [u8; 4] = str_field(value, "ip")
        .split('.')
        .map(|part| {
            part.parse::<u8>()
                .unwrap_or_else(|_| panic!("invalid IPv4 octet in {}", str_field(value, "ip")))
        })
        .collect::<Vec<u8>>()
        .try_into()
        .unwrap_or_else(|_| panic!("IPv4 must have exactly four octets"));
    i32::from_be_bytes(octets)
}

fn address(value: &Value) -> Address {
    if let Some(udp) = value.get("udp") {
        Address::Udp {
            ip: ip_field(udp),
            port: i32_field(udp, "port"),
        }
    } else if let Some(quic) = value.get("quic") {
        Address::Quic {
            ip: ip_field(quic),
            port: i32_field(quic, "port"),
        }
    } else {
        panic!("unsupported address {value}")
    }
}

fn address_list(value: &Value) -> AddressList {
    AddressList {
        addrs: arr_field(value, "addrs").iter().map(address).collect(),
        version: i32_field(value, "version"),
        reinit_date: i32_field(value, "reinit_date"),
        priority: i32_field(value, "priority"),
        expire_at: i32_field(value, "expire_at"),
    }
}

fn ed25519_key(hex_key: &str) -> PublicKey {
    PublicKey::Ed25519 {
        key: Int256(
            hex::decode(hex_key)
                .expect("public key must be hex")
                .try_into()
                .expect("public key must be 32 bytes"),
        ),
    }
}

fn dht_node(value: &Value) -> DhtNode {
    DhtNode {
        id: ed25519_key(str_field(value, "id")),
        addr_list: address_list(field(value, "addr_list")),
        version: i32_field(value, "version"),
        signature: bytes_field(value, "signature"),
    }
}

fn dht_node_list(value: &Value, key: &str) -> Vec<DhtNode> {
    arr_field(value, key).iter().map(dht_node).collect()
}

fn dht_value(value: &Value) -> DhtValue {
    let key_description = field(value, "key_description");
    let key = field(key_description, "key");
    DhtValue {
        key: DhtKeyDescription {
            key: DhtKey {
                id: int256_field(key, "id"),
                name: bytes_field(key, "name"),
                idx: i32_field(key, "idx"),
            },
            id: ed25519_key(str_field(key_description, "id")),
            update_rule: match str_field(key_description, "update_rule") {
                "signature" => DhtUpdateRule::Signature,
                "anybody" => DhtUpdateRule::Anybody,
                "overlayNodes" => DhtUpdateRule::OverlayNodes,
                other => panic!("unsupported update rule {other}"),
            },
            signature: bytes_field(key_description, "signature"),
        },
        value: bytes_field(value, "value"),
        ttl: i32_field(value, "ttl"),
        signature: bytes_field(value, "signature"),
    }
}

fn overlay_node(value: &Value) -> OverlayNode {
    OverlayNode {
        id: ed25519_key(str_field(value, "id")),
        overlay: int256_field(value, "overlay"),
        version: i32_field(value, "version"),
        signature: bytes_field(value, "signature"),
    }
}
