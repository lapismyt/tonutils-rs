//! Differential audit of hand-written network TL types against the pinned
//! upstream `ton_api.tl` schema.
//!
//! This is an offline protocol-divergence detector for the DHT, ADNL,
//! overlay, and QUIC surfaces. It checks, without any network access:
//!
//! 1. every constructor id hard-coded in `network.rs` / `adnl.rs` must be a
//!    constructor id computed from the upstream schema, so invented or stale
//!    ids fail the test;
//! 2. every schema-shaped doc comment must match the upstream definition
//!    after whitespace normalization, so fabricated fields or result types
//!    (`quic.query id:int256 data:bytes = quic.Query`) fail the test;
//! 3. the id normalization itself is calibrated against constructor ids
//!    observed on the live TON network, so a change in the computation can
//!    not silently pass.
//!
//! Constructor ids are computed with the ecosystem-standard normalization:
//! remove `(` and `)` characters, then CRC32 (IEEE) over the normalized
//! definition. This matches `tonutils-go` `tl.CRC` (`tl/loader.go`) and the
//! upstream TON TL parser output for the audited constructors; the
//! calibration table pins it to ids verified on real wire traffic.
//!
//! Scope is `network.rs` and `adnl.rs` (network protocol types). LiteAPI
//! types in `request.rs` / `response.rs` are audited by `schema_check.rs`.

use std::collections::{HashMap, HashSet};

const SCHEMA: &str = include_str!("schemas/ton_api.tl");
const SOURCES: [(&str, &str); 2] = [
    ("network.rs", include_str!("network.rs")),
    ("adnl.rs", include_str!("adnl.rs")),
];

/// Constructor ids observed on the live TON network (mainnet DHT/ADNL UDP
/// sessions). These pin the CRC normalization to wire truth: if the
/// computation drifts, this calibration fails before any other check.
const WIRE_CALIBRATION: &[(&str, u32)] = &[
    ("dht.nodes", 0x7974a0be),
    ("dht.findNode", 0x6ce2ce6b),
    ("adnl.packetContents", 0xd142cd89),
    ("adnl.addressList", 0x2227e658),
    ("overlay.nodes", 0xe487290e),
    ("adnl.message.nop", 0x17f8dfda),
    ("overlay.emptyCertificate", 0x32dabccf),
];

#[derive(Debug)]
struct SchemaCtor {
    definition: String,
    id: u32,
    result: String,
}

/// CRC32 (IEEE, reflected) as used for TL constructor ids.
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// Computes a TL constructor id from its definition using the ecosystem
/// normalization (parentheses removed before CRC32).
fn tl_constructor_id(definition: &str) -> u32 {
    let normalized: String = definition
        .chars()
        .filter(|ch| !matches!(ch, '(' | ')'))
        .collect();
    crc32_ieee(normalized.as_bytes())
}

/// Collapses whitespace and drops a trailing `;` so doc comments and schema
/// lines compare structurally.
fn normalize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches(';').to_string()
}

/// Parses the pinned upstream schema into `name -> SchemaCtor`, merging
/// multi-line constructor definitions.
fn parse_schema() -> HashMap<String, SchemaCtor> {
    let mut constructors = HashMap::new();
    let mut buffer = String::new();
    for raw in SCHEMA.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("---") {
            continue;
        }
        if buffer.is_empty() {
            buffer.push_str(line);
        } else {
            buffer.push(' ');
            buffer.push_str(line);
        }
        if !buffer.contains('=') {
            continue;
        }
        let definition = normalize(&buffer);
        buffer.clear();
        let Some(name) = definition.split_whitespace().next().filter(|token| {
            token.contains('.')
                && token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_'))
        }) else {
            continue;
        };
        let result = definition
            .split_once('=')
            .map(|(_, right)| right.trim().to_string())
            .unwrap_or_default();
        constructors.insert(
            name.to_string(),
            SchemaCtor {
                id: tl_constructor_id(&definition),
                definition,
                result,
            },
        );
    }
    assert!(
        constructors.len() > 600,
        "schema parse produced only {} constructors",
        constructors.len()
    );
    constructors
}

/// Collects `(line_number, id)` for every hard-coded TL constructor id in
/// an attribute position: either on a single-line `#[tl(..., id = 0x...)]`
/// or on the continuation line of a multi-line attribute.
fn collect_hardcoded_ids(source: &str) -> Vec<(usize, u32)> {
    source
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            let in_attribute = trimmed.starts_with("#[") || trimmed.starts_with("id = 0x");
            if !in_attribute {
                return None;
            }
            let pos = trimmed.find("id = 0x")?;
            let hex: String = trimmed[pos + "id = 0x".len()..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .take(8)
                .collect();
            if hex.is_empty() {
                return None;
            }
            Some((idx + 1, u32::from_str_radix(&hex, 16).expect("hex id")))
        })
        .collect()
}

/// Collects `(start_line, text)` for each block of consecutive `///` lines,
/// joining the lines with single spaces. Accumulation stops at the first
/// line that completes a schema-shaped definition (contains `=`), so prose
/// that follows a quoted schema line is not merged into the quote.
fn collect_doc_blocks(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let Some(first) = lines[idx].trim_start().strip_prefix("///") else {
            idx += 1;
            continue;
        };
        let start_line = idx + 1;
        let mut text = first.trim().to_string();
        idx += 1;
        while !text.contains('=') && idx < lines.len() {
            let Some(next) = lines[idx].trim_start().strip_prefix("///") else {
                break;
            };
            text.push(' ');
            text.push_str(next.trim());
            idx += 1;
        }
        blocks.push((start_line, text));
    }
    blocks
}

/// Returns the constructor name when a doc block carries a schema-shaped
/// definition (`name fields = Result`), i.e. it claims to quote `ton_api.tl`.
fn schema_shaped_name(text: &str) -> Option<&str> {
    let first = text.split_whitespace().next()?;
    if !first.contains('.')
        || !first
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_'))
    {
        return None;
    }
    text.contains('=').then_some(first)
}

/// Best-effort constructor name near a source line, for failure messages.
fn nearest_doc_name(source: &str, line_number: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let start = line_number.saturating_sub(1);
    let end = start.saturating_sub(12);
    (end..start).rev().find_map(|idx| {
        let text = lines.get(idx)?.trim_start().strip_prefix("///")?;
        schema_shaped_name(text.trim()).map(str::to_string)
    })
}

#[test]
fn schema_id_normalization_matches_live_wire_ids() {
    let schema = parse_schema();
    for (name, expected) in WIRE_CALIBRATION {
        let ctor = schema
            .get(*name)
            .unwrap_or_else(|| panic!("upstream schema no longer defines `{name}`"));
        assert_eq!(
            ctor.id, *expected,
            "TL id computation for `{name}` drifted: computed 0x{:08x}, \
             live wire value is 0x{expected:08x}",
            ctor.id
        );
    }
}

#[test]
fn hardcoded_constructor_ids_exist_in_upstream_schema() {
    let schema = parse_schema();
    let known: HashSet<u32> = schema.values().map(|ctor| ctor.id).collect();
    let mut audited = 0usize;
    for (file, source) in SOURCES {
        for (line, id) in collect_hardcoded_ids(source) {
            audited += 1;
            let context = nearest_doc_name(source, line).unwrap_or_default();
            assert!(
                known.contains(&id),
                "{file}:{line}: id 0x{id:08x} (near `{context}`) is not any \
                 constructor id computed from the pinned upstream ton_api.tl; \
                 it is invented or stale and would be rejected by real TON nodes"
            );
        }
    }
    assert!(
        audited >= 60,
        "only {audited} hard-coded ids audited; the source parser regressed"
    );
}

#[test]
fn doc_comments_match_upstream_schema_definitions() {
    let schema = parse_schema();
    let mut checked = 0usize;
    for (file, source) in SOURCES {
        for (line, text) in collect_doc_blocks(source) {
            let Some(name) = schema_shaped_name(&text) else {
                continue;
            };
            checked += 1;
            let Some(ctor) = schema.get(name) else {
                panic!(
                    "{file}:{line}: doc comment quotes `{name}` which does not \
                     exist in the pinned upstream ton_api.tl"
                );
            };
            assert_eq!(
                normalize(&text),
                ctor.definition,
                "{file}:{line}: doc comment for `{name}` diverges from the \
                 pinned upstream schema\n  ours:      {}\n  upstream:  {}",
                normalize(&text),
                ctor.definition
            );
            // Keep the result type referenced so schema updates that only
            // change the result name are still covered by the comparison.
            assert!(!ctor.result.is_empty(), "`{name}` has an empty result type");
        }
    }
    assert!(
        checked >= 35,
        "only {checked} schema-shaped doc comments audited; the doc parser regressed"
    );
}
