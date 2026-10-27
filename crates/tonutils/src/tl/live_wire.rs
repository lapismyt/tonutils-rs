//! Live mainnet wire fixtures: bytes captured from official TON nodes
//! through the opt-in ADNL capture facility (`tonutils_adnl`'s
//! `adnl::udp::capture`, armed by `ADNL_UDP_CAPTURE_FILE`).
//!
//! Unlike the cross-SDK fixtures these are not reproducible reference
//! serializations - they are what real nodes actually sent.  The tests
//! pin three properties: the ADNL datagram envelope framing, that our
//! `adnl.packetContents` decoder maps every observed constructor to the
//! variant that re-serializes to the same bytes, and that our overlay
//! decoders round-trip verbatim peer broadcast frames byte-for-byte.
//! Fixture provenance (harness, dates, per-case peers) lives in
//! `fixtures/cross_sdk/manifest.json` and in each file's cases.

use serde_json::Value;

use crate::tl::network::{
    OverlayBroadcast, OverlayBroadcastFec, OverlayMessage, PacketContents,
    TonNodeExternalMessageBroadcast,
};

/// Live wire fixtures bundled with the crate; the cross-SDK manifest
/// test matches this list against `fixtures/cross_sdk/manifest.json`.
pub(super) const LIVE_WIRE_FIXTURE_FILES: [(&str, &str); 3] = [
    (
        "live_adnl_udp_datagrams.json",
        include_str!("../../../../../fixtures/cross_sdk/live_adnl_udp_datagrams.json"),
    ),
    (
        "live_packet_contents.json",
        include_str!("../../../../../fixtures/cross_sdk/live_packet_contents.json"),
    ),
    (
        "live_overlay_broadcasts.json",
        include_str!("../../../../../fixtures/cross_sdk/live_overlay_broadcasts.json"),
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
    let mut names = std::collections::HashSet::new();
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

/// Overlay broadcast frames ride as verbatim `adnl.message.custom`
/// payloads: `[overlay.message][overlay id][inner TL]`.  These bytes
/// are the peer's own serialization, so the byte-exact round-trip
/// asserts our decoders against unmodified mainnet wire bytes.
#[test]
fn live_overlay_broadcast_cases_roundtrip_verbatim_peer_bytes() {
    for case in cases_of("live_overlay_broadcasts.json", LIVE_WIRE_FIXTURE_FILES[2].1) {
        let name = case["name"].as_str().expect("case name");
        let raw = raw_of("live_overlay_broadcasts.json", &case);
        let decoded: OverlayMessage = tl_proto::deserialize(&raw)
            .unwrap_or_else(|error| panic!("case {name} must decode: {error}"));
        let OverlayMessage::Message { overlay } = &decoded else {
            panic!("case {name}: the wrapper must be overlay.message");
        };
        assert_eq!(
            hex::encode(overlay.0),
            case["overlay_id_hex"]
                .as_str()
                .expect("case overlay_id_hex"),
            "case {name}: the overlay id must round-trip"
        );
        // The wrapper is a bare `[overlay.message][overlay id]` prefix of the
        // custom payload; the inner frame follows without a length prefix
        // (the `adnl.message.custom` `bytes` field carries the length one
        // layer up), mirroring `unwrap_overlay_payload` in tonutils-mempool.
        let wrapper = tl_proto::serialize(&decoded);
        assert_eq!(
            wrapper.len(),
            36,
            "case {name}: the wrapper must serialize to ctor + overlay id"
        );
        assert_eq!(
            wrapper,
            raw[..36],
            "case {name}: the wrapper must match the peer's prefix bytes"
        );
        let inner = &raw[36..];
        let label = case["inner_ctor_hex"]
            .as_str()
            .expect("case inner_ctor_hex");
        match label {
            // overlay.broadcastFec (0xbad7c36a)
            "6ac3d7ba" => {
                let fec: OverlayBroadcastFec = tl_proto::deserialize(inner)
                    .unwrap_or_else(|error| panic!("case {name} FEC must decode: {error}"));
                assert!(
                    fec.data_size > 0,
                    "case {name}: FEC data_size must be positive"
                );
                assert!(
                    !fec.data.is_empty(),
                    "case {name}: FEC symbols must be present"
                );
                assert!(
                    fec.date > 1_700_000_000,
                    "case {name}: FEC date must be a mainnet unix timestamp"
                );
                assert_eq!(
                    tl_proto::serialize(&fec),
                    inner,
                    "case {name}: the inner FEC frame must round-trip byte-exactly"
                );
            }
            // overlay.broadcast (0xb15a2b6b)
            "6b2b5ab1" => {
                let broadcast: OverlayBroadcast = tl_proto::deserialize(inner)
                    .unwrap_or_else(|error| panic!("case {name} broadcast must decode: {error}"));
                let OverlayBroadcast::Broadcast { data, date, .. } = &broadcast else {
                    panic!("case {name}: the inner frame must be overlay.broadcast");
                };
                assert!(
                    !data.is_empty(),
                    "case {name}: broadcast data must be present"
                );
                assert!(
                    *date > 1_700_000_000,
                    "case {name}: broadcast date must be a mainnet unix timestamp"
                );
                assert_eq!(
                    tl_proto::serialize(&broadcast),
                    inner,
                    "case {name}: the inner broadcast must round-trip byte-exactly"
                );
            }
            // tonNode.externalMessageBroadcast (0x3d1b1867)
            "67181b3d" => {
                let wrapped: TonNodeExternalMessageBroadcast = tl_proto::deserialize(inner)
                    .unwrap_or_else(|error| panic!("case {name} wrapper must decode: {error}"));
                assert_eq!(
                    &wrapped.message.data[..4],
                    &[0xb5, 0xee, 0x9c, 0x72],
                    "case {name}: the external message payload must start with the BoC magic"
                );
            }
            other => panic!("case {name}: unsupported inner constructor {other}"),
        }
    }
}
