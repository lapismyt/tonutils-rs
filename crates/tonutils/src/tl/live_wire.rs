#![expect(
    clippy::cast_possible_truncation,
    reason = "TON and ADNL wire fields are intentionally represented at protocol widths"
)]
//! Live mainnet wire fixtures: bytes captured from official TON nodes
//! through the opt-in ADNL capture facility (`tonutils::adnl`'s
//! `adnl::udp::capture`, armed by `ADNL_UDP_CAPTURE_FILE`).
//!
//! Unlike the cross-SDK fixtures these are not reproducible reference
//! serializations - they are what real nodes actually sent.  The tests
//! pin two properties: the ADNL datagram envelope framing, and that our
//! `adnl.packetContents` decoder maps every observed constructor to the
//! variant that re-serializes to the same bytes.
//! Fixture provenance (harness, dates, per-case peers) lives in
//! `fixtures/cross_sdk/manifest.json` and in each file's cases.

use std::collections::HashSet;

use crate::adnl::tl::PacketContents;

use serde_json::Value;

/// Live wire fixtures bundled with the crate; the cross-SDK manifest
/// test matches this list against `fixtures/cross_sdk/manifest.json`.
pub(super) const LIVE_WIRE_FIXTURE_FILES: [(&str, &str); 2] = [
    (
        "live_adnl_udp_datagrams.json",
        include_str!("../../../../fixtures/cross_sdk/live_adnl_udp_datagrams.json"),
    ),
    (
        "live_packet_contents.json",
        include_str!("../../../../fixtures/cross_sdk/live_packet_contents.json"),
    ),
];

fn cases_of(file: &str, content: &str) -> Vec<Value> {
    let fixture: Value = serde_json::from_str(content)
        .unwrap_or_else(|error| panic!("fixtures/cross_sdk/{file} must parse: {error}"));
    fixture["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("{file}: fixture cases must be an array"))
        .clone()
}

fn raw_of(file: &str, case: &Value) -> Vec<u8> {
    let name = case["name"].as_str().expect("case name");
    let raw = case["raw_hex"]
        .as_str()
        .unwrap_or_else(|| panic!("{file}: case {name} has no raw_hex"));
    hex::decode(raw).unwrap_or_else(|_| panic!("{file}: case {name} raw_hex must decode"))
}

#[test]
fn live_wire_cases_have_provenance_and_valid_hex() {
    let mut names = HashSet::new();
    for (file, content) in LIVE_WIRE_FIXTURE_FILES {
        let fixture: Value = serde_json::from_str(content).expect("live wire fixture must parse");
        assert!(
            fixture["captured_on"]
                .as_str()
                .is_some_and(|date| !date.is_empty()),
            "{file}: fixture must record captured_on"
        );
        for case in fixture["cases"].as_array().expect("fixture cases") {
            let name = case["name"].as_str().expect("case name");
            assert!(names.insert(name.to_owned()), "duplicate case name {name}");
            assert!(
                case["captured_at_unix"]
                    .as_u64()
                    .is_some_and(|at| at > 1_700_000_000),
                "{file}: case {name} must carry a recent capture timestamp"
            );
            let raw = case["raw_hex"].as_str().expect("case raw_hex");
            assert!(!raw.is_empty() && raw.len() % 2 == 0, "{file}: {name} hex");
            assert_eq!(
                case["length"].as_u64().expect("case length") as usize,
                raw.len() / 2,
                "{file}: case {name} length must match raw_hex"
            );
        }
    }
}

/// The datagram envelope: every captured wire datagram from an official
/// node is a 32-byte sender-or-channel prefix followed by the encrypted
/// body, larger than the prefix alone and never a STUN message (those
/// share the socket during address publication and are filtered).
#[test]
fn live_datagram_cases_pin_the_adnl_envelope_framing() {
    for case in cases_of("live_adnl_udp_datagrams.json", LIVE_WIRE_FIXTURE_FILES[0].1) {
        let name = case["name"].as_str().expect("case name");
        let raw = raw_of("live_adnl_udp_datagrams.json", &case);
        assert!(
            raw.len() > 32,
            "case {name}: a wire datagram must exceed the 32-byte prefix"
        );
        let prefix = case["prefix_hex"].as_str().expect("case prefix_hex");
        assert_eq!(
            hex::encode(&raw[..32]),
            prefix,
            "case {name}: the recorded prefix must be the datagram's first 32 bytes"
        );
        assert_ne!(
            &raw[4..8],
            &[0x21, 0x12, 0xa4, 0x42],
            "case {name}: STUN packets must not be captured as ADNL datagrams"
        );
    }
}

/// `adnl.packetContents`: our decoder must accept every body a mainnet
/// node sent, and re-serializing the decoded value must reproduce the
/// captured bytes exactly.  When the case carries a `message_ctor_hex`
/// label it must be the constructor of the decoded message variant -
/// the label is computed independently (TL layout arithmetic) at
/// extraction time, so a mismatch means our decode maps the
/// constructor to the wrong variant.
#[test]
fn live_packet_contents_cases_decode_and_reserialize_byte_exactly() {
    for case in cases_of("live_packet_contents.json", LIVE_WIRE_FIXTURE_FILES[1].1) {
        let name = case["name"].as_str().expect("case name");
        let raw = raw_of("live_packet_contents.json", &case);
        let decoded: PacketContents = tl_proto::deserialize(&raw)
            .unwrap_or_else(|error| panic!("case {name} must decode: {error}"));
        let re = tl_proto::serialize(&decoded);
        assert_eq!(
            re, raw,
            "case {name}: re-serialization must match the captured bytes"
        );
        let first = decoded
            .message
            .as_ref()
            .or_else(|| decoded.messages.as_ref().and_then(|list| list.first()));
        match (case["message_ctor_hex"].as_str(), first) {
            (Some(label), Some(message)) => {
                let serialized = tl_proto::serialize(message);
                assert_eq!(
                    hex::encode(&serialized[..4]),
                    label,
                    "case {name}: the label must match the decoded message variant"
                );
            }
            (None, _) => {}
            (Some(label), None) => {
                panic!("case {name}: label {label} but no decoded message");
            }
        }
    }
}
