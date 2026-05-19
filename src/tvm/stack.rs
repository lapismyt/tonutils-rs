//! TVM stack values used by smart-contract get-method calls.

use std::sync::Arc;

use anyhow::{Result, bail};
use num_bigint::BigInt;

use crate::tvm::{
    Builder, Cell, Slice,
    boc::{deserialize_boc, serialize_boc},
};

const TAG_NULL: u8 = 0;
const TAG_INT: u8 = 1;
const TAG_CELL: u8 = 2;
const TAG_SLICE: u8 = 3;
const TAG_TUPLE: u8 = 4;
const TAG_LIST: u8 = 5;
const TAG_UNSUPPORTED: u8 = 255;
const MAX_STACK_DEPTH: usize = 0xFF_FFFF;

/// A minimal owned TVM stack entry representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TvmStackEntry {
    Null,
    Int(BigInt),
    Cell(Arc<Cell>),
    Slice(Arc<Cell>),
    Tuple(Vec<TvmStackEntry>),
    List(Vec<TvmStackEntry>),
    Unsupported(Vec<u8>),
}

impl TvmStackEntry {
    pub fn int(value: impl Into<BigInt>) -> Self {
        Self::Int(value.into())
    }
}

/// A TVM stack container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TvmStack {
    entries: Vec<TvmStackEntry>,
}

impl TvmStack {
    pub fn new(entries: Vec<TvmStackEntry>) -> Self {
        Self { entries }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[TvmStackEntry] {
        &self.entries
    }

    pub fn push(&mut self, entry: TvmStackEntry) {
        self.entries.push(entry);
    }

    pub fn to_boc(&self) -> Result<Vec<u8>> {
        serialize_boc(&self.to_cell()?, false)
    }

    pub fn from_boc(bytes: &[u8]) -> Result<Self> {
        let cell = deserialize_boc(bytes)?;
        Self::from_cell(cell)
    }

    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        encode_entries(&self.entries)
    }

    pub fn from_cell(cell: Arc<Cell>) -> Result<Self> {
        let entries = decode_entries(cell)?;
        Ok(Self { entries })
    }
}

fn encode_entries(entries: &[TvmStackEntry]) -> Result<Arc<Cell>> {
    if entries.len() > MAX_STACK_DEPTH {
        bail!("TVM stack is too large");
    }

    let mut builder = Builder::new();
    builder.store_uint_custom::<u32>(entries.len() as u32, 24)?;
    if !entries.is_empty() {
        builder.store_ref(encode_entry_chain(entries)?)?;
    }
    builder.build()
}

fn encode_entry_chain(entries: &[TvmStackEntry]) -> Result<Arc<Cell>> {
    let count = entries.len().min(3);
    let mut builder = Builder::new();
    builder.store_uint::<u8>(count as u8)?;
    builder.store_bit(entries.len() > count)?;
    for entry in &entries[..count] {
        builder.store_ref(encode_entry(entry)?)?;
    }
    if entries.len() > count {
        builder.store_ref(encode_entry_chain(&entries[count..])?)?;
    }
    builder.build()
}

fn encode_entry(entry: &TvmStackEntry) -> Result<Arc<Cell>> {
    let mut builder = Builder::new();
    match entry {
        TvmStackEntry::Null => {
            builder.store_byte(TAG_NULL)?;
        }
        TvmStackEntry::Int(value) => {
            builder.store_byte(TAG_INT)?;
            let bytes = value.to_signed_bytes_be();
            if bytes.len() > u16::MAX as usize {
                bail!("TVM stack integer is too large");
            }
            builder.store_uint::<u16>(bytes.len() as u16)?;
            builder.store_bytes(&bytes)?;
        }
        TvmStackEntry::Cell(cell) => {
            builder.store_byte(TAG_CELL)?;
            builder.store_ref(cell.clone())?;
        }
        TvmStackEntry::Slice(cell) => {
            builder.store_byte(TAG_SLICE)?;
            builder.store_ref(cell.clone())?;
        }
        TvmStackEntry::Tuple(entries) => {
            builder.store_byte(TAG_TUPLE)?;
            builder.store_ref(encode_entries(entries)?)?;
        }
        TvmStackEntry::List(entries) => {
            builder.store_byte(TAG_LIST)?;
            builder.store_ref(encode_entries(entries)?)?;
        }
        TvmStackEntry::Unsupported(bytes) => {
            if bytes.len() > 127 {
                bail!("Unsupported stack payload is too large");
            }
            builder.store_byte(TAG_UNSUPPORTED)?;
            builder.store_uint::<u8>(bytes.len() as u8)?;
            builder.store_bytes(bytes)?;
        }
    }
    builder.build()
}

fn decode_entries(cell: Arc<Cell>) -> Result<Vec<TvmStackEntry>> {
    let mut slice = Slice::new(cell);
    let len = slice.load_uint_custom::<u32>(24)? as usize;
    let mut entries = if len == 0 {
        Vec::new()
    } else {
        decode_entry_chain(slice.load_reference()?, len)?
    };
    if entries.len() != len {
        bail!("TVM stack entry count mismatch");
    }
    entries.truncate(len);
    Ok(entries)
}

fn decode_entry_chain(cell: Arc<Cell>, remaining: usize) -> Result<Vec<TvmStackEntry>> {
    let mut slice = Slice::new(cell);
    let count = slice.load_uint::<u8>()? as usize;
    let has_next = slice.load_bit()?;
    if count > 3 {
        bail!("TVM stack entry chain node is too large");
    }
    if count > remaining {
        bail!("TVM stack entry chain exceeds expected length");
    }

    let mut entries = Vec::with_capacity(remaining);
    for _ in 0..count {
        entries.push(decode_entry(slice.load_reference()?)?);
    }
    if has_next {
        entries.extend(decode_entry_chain(
            slice.load_reference()?,
            remaining - count,
        )?);
    }
    Ok(entries)
}

fn decode_entry(cell: Arc<Cell>) -> Result<TvmStackEntry> {
    let mut slice = Slice::new(cell);
    let tag = slice.load_byte()?;
    match tag {
        TAG_NULL => Ok(TvmStackEntry::Null),
        TAG_INT => {
            let len = slice.load_uint::<u16>()? as usize;
            Ok(TvmStackEntry::Int(BigInt::from_signed_bytes_be(
                &slice.load_bytes(len)?,
            )))
        }
        TAG_CELL => Ok(TvmStackEntry::Cell(slice.load_reference()?)),
        TAG_SLICE => Ok(TvmStackEntry::Slice(slice.load_reference()?)),
        TAG_TUPLE => Ok(TvmStackEntry::Tuple(decode_entries(
            slice.load_reference()?,
        )?)),
        TAG_LIST => Ok(TvmStackEntry::List(decode_entries(
            slice.load_reference()?,
        )?)),
        TAG_UNSUPPORTED => {
            let len = slice.load_uint::<u8>()? as usize;
            Ok(TvmStackEntry::Unsupported(slice.load_bytes(len)?))
        }
        _ => bail!("Unknown TVM stack entry tag {}", tag),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tvm::CellBuilder;

    #[test]
    fn test_stack_roundtrip_simple_values() {
        let stack = TvmStack::new(vec![
            TvmStackEntry::Null,
            TvmStackEntry::int(-42),
            TvmStackEntry::Unsupported(vec![1, 2, 3]),
        ]);

        let boc = stack.to_boc().unwrap();
        let decoded = TvmStack::from_boc(&boc).unwrap();

        assert_eq!(decoded, stack);
    }

    #[test]
    fn test_empty_stack_uses_ton_vm_stack_depth_width() {
        let cell = TvmStack::empty().to_cell().unwrap();
        assert_eq!(cell.bit_len(), 24);

        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_uint_custom::<u32>(24).unwrap(), 0);
        assert!(slice.is_empty());
    }

    #[test]
    fn test_stack_roundtrip_cell_and_tuple() {
        let mut builder = CellBuilder::new();
        builder.store_u32(0x12345678).unwrap();
        let cell = builder.build().unwrap();

        let stack = TvmStack::new(vec![
            TvmStackEntry::Cell(cell.clone()),
            TvmStackEntry::Tuple(vec![TvmStackEntry::Slice(cell)]),
        ]);

        let decoded = TvmStack::from_boc(&stack.to_boc().unwrap()).unwrap();

        assert_eq!(decoded, stack);
    }

    #[test]
    fn test_stack_roundtrip_big_integer_and_many_entries() {
        let big = BigInt::parse_bytes(b"123456789012345678901234567890", 10).unwrap();
        let mut entries = vec![TvmStackEntry::Int(big)];
        for i in 0..8 {
            entries.push(TvmStackEntry::int(i));
        }
        let stack = TvmStack::new(entries);

        let decoded = TvmStack::from_boc(&stack.to_boc().unwrap()).unwrap();

        assert_eq!(decoded, stack);
    }

    #[test]
    fn checked_stack_fixtures_decode_and_reserialize_canonically() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/tvm/stack.json")).unwrap();
        assert_eq!(
            fixture["source"],
            "synthetic offline fixture generated by tonutils-rs stack codec"
        );
        let vectors = fixture["vectors"].as_array().unwrap();
        assert_eq!(vectors.len(), 6);
        let captured = fixture["captured_or_opt_in"].as_array().unwrap();
        assert!(!captured.is_empty());
        for vector in captured {
            for key in [
                "source_sdk_or_tool",
                "source_version_or_commit",
                "network",
                "endpoint",
                "block_id",
                "account",
                "method",
                "input_stack_json",
                "params_boc_hex",
                "params_root_hash",
                "exit_code",
                "result_boc_hex",
                "result_root_hash",
                "decoded_result",
                "compat_reference",
                "capture_command",
            ] {
                assert!(
                    vector.as_object().unwrap().contains_key(key),
                    "captured stack vector missing {key}"
                );
            }
        }
        for vector in fixture["cross_sdk_vectors"].as_array().unwrap() {
            let local = vector["params_boc_hex"].as_str().unwrap();
            if let Some(reference) = vector["reference_params_boc_hex"].as_str() {
                assert_eq!(local, reference, "{}", vector["name"]);
            }
            if let Some(result_boc) = vector["result_boc_hex"].as_str() {
                let stack = TvmStack::from_boc(&hex::decode(result_boc).unwrap()).unwrap();
                assert_eq!(hex::encode(stack.to_boc().unwrap()), result_boc);
            }
        }

        for vector in vectors {
            let name = vector["name"].as_str().unwrap();
            let boc_hex = vector["input_stack_boc_hex"].as_str().unwrap();
            let boc = hex::decode(boc_hex).unwrap();
            let decoded = TvmStack::from_boc(&boc).unwrap();
            assert_eq!(decoded, expected_stack_fixture(name));
            assert_eq!(hex::encode(decoded.to_boc().unwrap()), boc_hex);
            assert_eq!(
                hex::encode(decoded.to_cell().unwrap().hash()),
                vector["root_hash"].as_str().unwrap()
            );
            assert_eq!(
                stack_entries_fixture_value(decoded.entries()),
                vector["decoded_entries"]
            );
        }
    }

    fn expected_stack_fixture(name: &str) -> TvmStack {
        let mut builder = CellBuilder::new();
        builder.store_u32(0x12345678).unwrap();
        let cell = builder.build().unwrap();
        let huge =
            BigInt::parse_bytes(b"12345678901234567890123456789012345678901234567890", 10).unwrap();
        match name {
            "scalar_non_empty" => TvmStack::new(vec![TvmStackEntry::Null, TvmStackEntry::int(-42)]),
            "more_than_four_entries" => TvmStack::new((0..7).map(TvmStackEntry::int).collect()),
            "nested_tuple_list" => TvmStack::new(vec![TvmStackEntry::Tuple(vec![
                TvmStackEntry::int(1),
                TvmStackEntry::List(vec![TvmStackEntry::Null, TvmStackEntry::int(2)]),
            ])]),
            "huge_integer" => TvmStack::new(vec![TvmStackEntry::Int(huge)]),
            "cell_and_slice" => TvmStack::new(vec![
                TvmStackEntry::Cell(cell.clone()),
                TvmStackEntry::Slice(cell),
            ]),
            "unsupported_raw" => {
                TvmStack::new(vec![TvmStackEntry::Unsupported(vec![0x0a, 0x0b, 0x0c])])
            }
            other => panic!("unexpected stack fixture {other}"),
        }
    }

    fn stack_entries_fixture_value(entries: &[TvmStackEntry]) -> serde_json::Value {
        serde_json::Value::Array(entries.iter().map(stack_entry_fixture_value).collect())
    }

    fn stack_entry_fixture_value(entry: &TvmStackEntry) -> serde_json::Value {
        match entry {
            TvmStackEntry::Null => serde_json::json!({ "type": "null" }),
            TvmStackEntry::Int(value) => {
                serde_json::json!({ "type": "int", "value": value.to_str_radix(10) })
            }
            TvmStackEntry::Cell(cell) => serde_json::json!({
                "type": "cell",
                "boc": hex::encode(serialize_boc(cell, false).unwrap())
            }),
            TvmStackEntry::Slice(cell) => serde_json::json!({
                "type": "slice",
                "boc": hex::encode(serialize_boc(cell, false).unwrap())
            }),
            TvmStackEntry::Tuple(entries) => serde_json::json!({
                "type": "tuple",
                "entries": stack_entries_fixture_value(entries)
            }),
            TvmStackEntry::List(entries) => serde_json::json!({
                "type": "list",
                "entries": stack_entries_fixture_value(entries)
            }),
            TvmStackEntry::Unsupported(bytes) => {
                serde_json::json!({ "type": "unsupported", "raw": hex::encode(bytes) })
            }
        }
    }
}
