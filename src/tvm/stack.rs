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
    if entries.len() > u16::MAX as usize {
        bail!("TVM stack is too large");
    }

    let mut builder = Builder::new();
    builder.store_uint(entries.len() as u64, 16)?;
    if !entries.is_empty() {
        builder.store_ref(encode_entry_chain(entries)?)?;
    }
    builder.build()
}

fn encode_entry_chain(entries: &[TvmStackEntry]) -> Result<Arc<Cell>> {
    let count = entries.len().min(3);
    let mut builder = Builder::new();
    builder.store_uint(count as u64, 8)?;
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
            builder.store_uint(bytes.len() as u64, 16)?;
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
            builder.store_uint(bytes.len() as u64, 8)?;
            builder.store_bytes(bytes)?;
        }
    }
    builder.build()
}

fn decode_entries(cell: Arc<Cell>) -> Result<Vec<TvmStackEntry>> {
    let mut slice = Slice::new(cell);
    let len = slice.load_uint(16)? as usize;
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
    let count = slice.load_uint(8)? as usize;
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
            let len = slice.load_uint(16)? as usize;
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
            let len = slice.load_uint(8)? as usize;
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
}
