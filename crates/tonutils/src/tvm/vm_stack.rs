#![expect(
    clippy::cast_possible_truncation,
    reason = "TON stack tags and slice ranges are intentionally represented at protocol widths"
)]
//! Upstream-compatible TVM stack codec.
//!
//! [`VmStack`] serializes the standard `VmStack` layout defined by
//! `crypto/vm/stack.cpp` in the reference TON implementation. Unlike the
//! high-level [`TvmStack`], which preserves provider
//! payloads with a crate-specific tag format, this codec matches the wire
//! format consumed and produced by `libemulator` and standard get-method
//! result BoCs.
//!
//! Values are stored bottom-to-top. The root cell contains a 24-bit depth
//! followed by the topmost value and a reference to the rest of the stack;
//! each lower entry is a cons cell holding the previous rest reference and the
//! entry value.
//!
//! Supported tags:
//!
//! | Tag | Value |
//! | --- | --- |
//! | `00` | `null` |
//! | `01` + 64 bits | tiny signed integer |
//! | `02ff` | `NaN` |
//! | `0201_` + 257 bits | signed integer |
//! | `03` + `^Cell` | cell |
//! | `04` + cell slice range | slice |
//! | `05` + `^Cell` | builder snapshot |
//! | `07` + 16-bit length + tuple refs | tuple |
//!
//! Continuations (`06`) are not representable and produce
//! [`TvmError::Stack`] instead of being silently
//! dropped. A decoded builder is represented by its finalized cell snapshot.

use std::sync::Arc;

use num_bigint::BigInt;

use crate::tvm::{
    Builder, Cell, Slice, TvmError, TvmResult as Result, boc::deserialize_boc, boc::serialize_boc,
    error::stack_bail as bail, stack::TvmStack, stack::TvmStackEntry,
};

/// Maximum number of values accepted by the upstream stack codec.
pub const MAX_VM_STACK_DEPTH: usize = 1024;

/// Maximum tuple length accepted by the upstream TVM stack codec.
pub const MAX_VM_TUPLE_LEN: usize = 255;

const TAG_NULL: u8 = 0;
const TAG_TINY_INT: u8 = 1;
const TAG_INT_OR_NAN: u8 = 2;
const TAG_CELL: u8 = 3;
const TAG_SLICE: u8 = 4;
const TAG_BUILDER: u8 = 5;
const TAG_CONTINUATION: u8 = 6;
const TAG_TUPLE: u8 = 7;

/// A standard TVM stack value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmStackValue {
    /// `vm_stk_null#00`.
    Null,
    /// `vm_stk_nan#02ff`.
    Nan,
    /// `vm_stk_tinyint#01` or `vm_stk_int#0201_`, stored as a signed integer.
    Int(BigInt),
    /// `vm_stk_cell#03`.
    Cell(Arc<Cell>),
    /// `vm_stk_slice#04`, preserving its bit and reference range.
    Slice(VmSlice),
    /// `vm_stk_builder#05`, represented by the builder's finalized cell snapshot.
    Builder(Arc<Cell>),
    /// `vm_stk_tuple#07`.
    Tuple(Vec<VmStackValue>),
}

impl VmStackValue {
    /// Creates an integer value from any type convertible into [`BigInt`].
    pub fn int(value: impl Into<BigInt>) -> Self {
        Self::Int(value.into())
    }

    /// Serializes a standalone value into a cell.
    ///
    /// This is the format used by libemulator's previous-block-info setter,
    /// which takes a single serialized `VmStackValue` rather than a full stack.
    ///
    /// # Errors
    ///
    /// Returns [`TvmError::Stack`] when the value
    /// is not representable in the standard stack format.
    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        let mut builder = Builder::new();
        serialize_value(self, &mut builder)?;
        builder.build()
    }

    /// Decodes a standalone value from a cell.
    ///
    /// # Errors
    ///
    /// Returns [`TvmError::Stack`] when the cell
    /// is malformed or contains trailing data.
    pub fn from_cell(cell: Arc<Cell>) -> Result<Self> {
        deserialize_cell_exact(cell)
    }

    /// Serializes a standalone value into a Bag of Cells.
    ///
    /// # Errors
    ///
    /// Propagates value encoding and BoC serialization errors.
    pub fn to_boc(&self) -> Result<Vec<u8>> {
        serialize_boc(&self.to_cell()?, false)
    }

    /// Decodes a standalone value from a Bag of Cells.
    ///
    /// # Errors
    ///
    /// Propagates BoC decoding and value parsing errors.
    pub fn from_boc(bytes: &[u8]) -> Result<Self> {
        Self::from_cell(deserialize_boc(bytes)?)
    }
}

/// A cell slice view with explicit bit and reference ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmSlice {
    cell: Arc<Cell>,
    start_bits: usize,
    end_bits: usize,
    start_refs: usize,
    end_refs: usize,
}

impl VmSlice {
    /// Creates a slice view over a cell range.
    ///
    /// # Errors
    ///
    /// Returns [`TvmError::Stack`] when the range
    /// is inverted, exceeds the cell contents, or references more than four
    /// cells.
    pub fn new(
        cell: Arc<Cell>,
        start_bits: usize,
        end_bits: usize,
        start_refs: usize,
        end_refs: usize,
    ) -> Result<Self> {
        if start_bits > end_bits {
            bail!("VM slice start bit offset exceeds its end offset");
        }
        if end_bits > cell.bit_len() {
            bail!(
                "VM slice end bit offset {} exceeds cell bit length {}",
                end_bits,
                cell.bit_len()
            );
        }
        if start_refs > end_refs {
            bail!("VM slice start reference offset exceeds its end offset");
        }
        if end_refs > cell.references().len() {
            bail!(
                "VM slice end reference offset {} exceeds cell reference count {}",
                end_refs,
                cell.references().len()
            );
        }
        Ok(Self {
            cell,
            start_bits,
            end_bits,
            start_refs,
            end_refs,
        })
    }

    /// Creates a full-range slice over an entire cell.
    pub fn from_cell(cell: Arc<Cell>) -> Self {
        let end_bits = cell.bit_len();
        let end_refs = cell.references().len();
        Self {
            cell,
            start_bits: 0,
            end_bits,
            start_refs: 0,
            end_refs,
        }
    }

    /// Creates a full-range slice view from an existing [`Slice`] position.
    pub fn from_slice(slice: &Slice) -> Self {
        let cell = slice.cell().clone();
        Self {
            start_bits: slice.bit_position(),
            end_bits: cell.bit_len(),
            start_refs: slice.ref_position(),
            end_refs: cell.references().len(),
            cell,
        }
    }

    /// Returns the underlying cell.
    pub fn cell(&self) -> &Arc<Cell> {
        &self.cell
    }

    /// Returns the first included bit offset.
    pub fn start_bits(&self) -> usize {
        self.start_bits
    }

    /// Returns the exclusive ending bit offset.
    pub fn end_bits(&self) -> usize {
        self.end_bits
    }

    /// Returns the first included reference offset.
    pub fn start_refs(&self) -> usize {
        self.start_refs
    }

    /// Returns the exclusive ending reference offset.
    pub fn end_refs(&self) -> usize {
        self.end_refs
    }

    /// Builds a [`Slice`] positioned at the start of this view.
    ///
    /// The returned slice is bounded by the underlying cell, not by
    /// [`end_bits`](Self::end_bits) or [`end_refs`](Self::end_refs), because
    /// [`Slice`] does not carry an end range. Use the accessors when the exact
    /// range matters.
    pub fn to_slice(&self) -> Result<Slice> {
        let mut slice = Slice::new(self.cell.clone());
        slice.skip_bits(self.start_bits)?;
        slice.skip_refs(self.start_refs)?;
        Ok(slice)
    }
}

/// A standard upstream-compatible TVM stack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VmStack {
    values: Vec<VmStackValue>,
}

impl VmStack {
    /// Creates a stack from values ordered bottom-to-top.
    pub fn new(values: Vec<VmStackValue>) -> Self {
        Self { values }
    }

    /// Returns the stack values ordered bottom-to-top.
    pub fn values(&self) -> &[VmStackValue] {
        &self.values
    }

    /// Consumes the stack and returns values ordered bottom-to-top.
    pub fn into_values(self) -> Vec<VmStackValue> {
        self.values
    }

    /// Returns the number of values on the stack.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns `true` when the stack has no values.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Pushes a value on top of the stack.
    pub fn push(&mut self, value: VmStackValue) {
        self.values.push(value);
    }

    /// Serializes the stack into its root cell.
    ///
    /// # Errors
    ///
    /// Returns [`TvmError::Stack`] when the stack
    /// exceeds [`MAX_VM_STACK_DEPTH`], a tuple exceeds
    /// [`MAX_VM_TUPLE_LEN`], or a value is not representable in the standard
    /// stack format.
    pub fn to_cell(&self) -> Result<Arc<Cell>> {
        let len = self.values.len();
        if len > MAX_VM_STACK_DEPTH {
            bail!("VM stack depth {} exceeds {}", len, MAX_VM_STACK_DEPTH);
        }

        let mut root = Builder::new();
        root.store_uint_custom::<u32>(len as u32, 24)?;
        if len == 0 {
            return root.build();
        }

        let mut rest = Builder::new().build()?;
        for value in &self.values[..len - 1] {
            let mut node = Builder::new();
            node.store_ref(rest)?;
            serialize_value(value, &mut node)?;
            rest = node.build()?;
        }

        root.store_ref(rest)?;
        serialize_value(&self.values[len - 1], &mut root)?;
        root.build()
    }

    /// Decodes a stack from its root cell.
    ///
    /// # Errors
    ///
    /// Returns [`TvmError::Stack`] when the cell is
    /// not a valid standard stack, contains a continuation or unknown tag, or
    /// has trailing data.
    pub fn from_cell(cell: Arc<Cell>) -> Result<Self> {
        let mut root = Slice::new(cell);
        let len = root.load_uint_custom::<u32>(24)? as usize;
        if len > MAX_VM_STACK_DEPTH {
            bail!("VM stack depth {} exceeds {}", len, MAX_VM_STACK_DEPTH);
        }
        if len == 0 {
            if !root.is_empty() {
                bail!("Trailing data after empty VM stack");
            }
            return Ok(Self::default());
        }

        let mut rest = root.load_reference()?;
        let top = deserialize_value(&mut root)?;
        if !root.is_empty() {
            bail!("Trailing data after VM stack root value");
        }

        let mut values = Vec::with_capacity(len);
        values.push(top);
        for _ in 0..len - 1 {
            let mut node = Slice::new(rest);
            rest = node.load_reference()?;
            values.push(deserialize_value(&mut node)?);
            if !node.is_empty() {
                bail!("Trailing data after VM stack cons value");
            }
        }
        if !Slice::new(rest).is_empty() {
            bail!("Trailing data after VM stack list terminator");
        }

        values.reverse();
        Ok(Self { values })
    }

    /// Serializes the stack to a Bag-of-Cells byte vector.
    ///
    /// # Errors
    ///
    /// Propagates stack encoding and BoC serialization errors.
    pub fn to_boc(&self) -> Result<Vec<u8>> {
        serialize_boc(&self.to_cell()?, false)
    }

    /// Decodes a stack from a Bag-of-Cells byte vector.
    ///
    /// # Errors
    ///
    /// Propagates BoC decoding and stack parsing errors.
    pub fn from_boc(bytes: &[u8]) -> Result<Self> {
        Self::from_cell(deserialize_boc(bytes)?)
    }
}

impl TryFrom<TvmStack> for VmStack {
    type Error = TvmError;

    fn try_from(value: TvmStack) -> Result<Self> {
        let mut values = Vec::with_capacity(value.entries().len());
        for entry in value.entries() {
            values.push(convert_entry_from_high_level(entry)?);
        }
        Ok(Self::new(values))
    }
}

impl TryFrom<&TvmStack> for VmStack {
    type Error = TvmError;

    fn try_from(value: &TvmStack) -> Result<Self> {
        let mut values = Vec::with_capacity(value.entries().len());
        for entry in value.entries() {
            values.push(convert_entry_from_high_level(entry)?);
        }
        Ok(Self::new(values))
    }
}

impl TryFrom<VmStack> for TvmStack {
    type Error = TvmError;

    fn try_from(value: VmStack) -> Result<Self> {
        let mut entries = Vec::with_capacity(value.len());
        for entry in value.into_values() {
            entries.push(convert_value_to_high_level(entry)?);
        }
        Ok(Self::new(entries))
    }
}

impl TryFrom<&VmStack> for TvmStack {
    type Error = TvmError;

    fn try_from(value: &VmStack) -> Result<Self> {
        let mut entries = Vec::with_capacity(value.len());
        for entry in value.values() {
            entries.push(convert_value_to_high_level(entry.clone())?);
        }
        Ok(Self::new(entries))
    }
}

fn convert_entry_from_high_level(entry: &TvmStackEntry) -> Result<VmStackValue> {
    match entry {
        TvmStackEntry::Null => Ok(VmStackValue::Null),
        TvmStackEntry::Int(value) => Ok(VmStackValue::Int(value.clone())),
        TvmStackEntry::Cell(cell) => Ok(VmStackValue::Cell(cell.clone())),
        TvmStackEntry::Slice(cell) => Ok(VmStackValue::Slice(VmSlice::from_cell(cell.clone()))),
        TvmStackEntry::Tuple(entries) | TvmStackEntry::List(entries) => {
            let mut values = Vec::with_capacity(entries.len());
            for entry in entries {
                values.push(convert_entry_from_high_level(entry)?);
            }
            Ok(VmStackValue::Tuple(values))
        }
        TvmStackEntry::Unsupported(_) => {
            bail!("Unsupported stack payload cannot be represented as a standard VM stack value")
        }
    }
}

fn convert_value_to_high_level(value: VmStackValue) -> Result<TvmStackEntry> {
    match value {
        VmStackValue::Null => Ok(TvmStackEntry::Null),
        VmStackValue::Int(value) => Ok(TvmStackEntry::Int(value)),
        VmStackValue::Cell(cell) | VmStackValue::Builder(cell) => Ok(TvmStackEntry::Cell(cell)),
        VmStackValue::Slice(slice) => {
            if slice.start_bits() != 0
                || slice.end_bits() != slice.cell().bit_len()
                || slice.start_refs() != 0
                || slice.end_refs() != slice.cell().references().len()
            {
                bail!("Standard VM slice with a partial range cannot be represented by TvmStack");
            }
            Ok(TvmStackEntry::Slice(slice.cell().clone()))
        }
        VmStackValue::Tuple(values) => {
            let mut entries = Vec::with_capacity(values.len());
            for value in values {
                entries.push(convert_value_to_high_level(value)?);
            }
            Ok(TvmStackEntry::Tuple(entries))
        }
        VmStackValue::Nan => {
            bail!("TVM NaN stack value cannot be represented by TvmStack")
        }
    }
}

fn serialize_value(value: &VmStackValue, builder: &mut Builder) -> Result<()> {
    match value {
        VmStackValue::Null => {
            builder.store_u8(TAG_NULL)?;
        }
        VmStackValue::Nan => {
            builder.store_u16(0x02ff)?;
        }
        VmStackValue::Int(value) => {
            if i64::try_from(value).is_ok() {
                builder.store_u8(TAG_TINY_INT)?;
                builder.store_big_int(value, 64)?;
            } else {
                builder.store_uint_custom::<u16>(0x0100, 15)?;
                builder.store_big_int(value, 257)?;
            }
        }
        VmStackValue::Cell(cell) => {
            builder.store_u8(TAG_CELL)?;
            builder.store_ref(cell.clone())?;
        }
        VmStackValue::Slice(slice) => {
            builder.store_u8(TAG_SLICE)?;
            builder.store_ref(slice.cell.clone())?;
            builder.store_uint_custom::<u16>(slice.start_bits as u16, 10)?;
            builder.store_uint_custom::<u16>(slice.end_bits as u16, 10)?;
            builder.store_uint_custom::<u8>(slice.start_refs as u8, 3)?;
            builder.store_uint_custom::<u8>(slice.end_refs as u8, 3)?;
        }
        VmStackValue::Builder(cell) => {
            builder.store_u8(TAG_BUILDER)?;
            builder.store_ref(cell.clone())?;
        }
        VmStackValue::Tuple(values) => {
            serialize_tuple(values, builder)?;
        }
    }
    Ok(())
}

fn serialize_tuple(values: &[VmStackValue], builder: &mut Builder) -> Result<()> {
    if values.len() > MAX_VM_TUPLE_LEN {
        bail!(
            "VM tuple length {} exceeds {}",
            values.len(),
            MAX_VM_TUPLE_LEN
        );
    }

    let mut head: Option<Arc<Cell>> = None;
    let mut tail: Option<Arc<Cell>> = None;

    for (index, value) in values.iter().enumerate() {
        std::mem::swap(&mut head, &mut tail);
        if index > 1 {
            let previous_head = head
                .take()
                .ok_or_else(|| TvmError::Stack("VM tuple encoder lost its head cell".to_owned()))?;
            let previous_tail = tail
                .take()
                .ok_or_else(|| TvmError::Stack("VM tuple encoder lost its tail cell".to_owned()))?;
            let mut pair = Builder::new();
            pair.store_ref(previous_tail)?;
            pair.store_ref(previous_head)?;
            head = Some(pair.build()?);
        }

        let mut node = Builder::new();
        serialize_value(value, &mut node)?;
        tail = Some(node.build()?);
    }

    builder.store_u8(TAG_TUPLE)?;
    builder.store_u16(values.len() as u16)?;
    if let Some(head) = head {
        builder.store_ref(head)?;
    }
    if let Some(tail) = tail {
        builder.store_ref(tail)?;
    }
    Ok(())
}

fn deserialize_value(slice: &mut Slice) -> Result<VmStackValue> {
    let tag = slice.load_u8()?;
    match tag {
        TAG_NULL => Ok(VmStackValue::Null),
        TAG_TINY_INT => Ok(VmStackValue::Int(slice.load_big_int(64)?)),
        TAG_INT_OR_NAN => {
            let mut probe = slice.clone_from_current();
            if probe.load_u8()? == 0xff {
                slice.skip_bits(8)?;
                return Ok(VmStackValue::Nan);
            }
            let tag_tail = slice.load_uint_custom::<u8>(7)?;
            if tag_tail != 0 {
                bail!("Invalid standard VM integer tag");
            }
            Ok(VmStackValue::Int(slice.load_big_int(257)?))
        }
        TAG_CELL => Ok(VmStackValue::Cell(slice.load_reference()?)),
        TAG_SLICE => {
            let cell = slice.load_reference()?;
            let start_bits = slice.load_uint_custom::<u16>(10)? as usize;
            let end_bits = slice.load_uint_custom::<u16>(10)? as usize;
            let start_refs = slice.load_uint_custom::<u8>(3)? as usize;
            let end_refs = slice.load_uint_custom::<u8>(3)? as usize;
            Ok(VmStackValue::Slice(VmSlice::new(
                cell, start_bits, end_bits, start_refs, end_refs,
            )?))
        }
        TAG_BUILDER => Ok(VmStackValue::Builder(slice.load_reference()?)),
        TAG_CONTINUATION => bail!("VM stack continuations are not supported"),
        TAG_TUPLE => deserialize_tuple(slice),
        other => bail!("Unknown standard VM stack value tag {}", other),
    }
}

fn deserialize_cell_exact(cell: Arc<Cell>) -> Result<VmStackValue> {
    let mut slice = Slice::new(cell);
    let value = deserialize_value(&mut slice)?;
    if !slice.is_empty() {
        bail!("Trailing data after nested VM stack value");
    }
    Ok(value)
}

fn deserialize_tuple(slice: &mut Slice) -> Result<VmStackValue> {
    let len = slice.load_u16()? as usize;
    if len > MAX_VM_TUPLE_LEN {
        bail!("VM tuple length {} exceeds {}", len, MAX_VM_TUPLE_LEN);
    }
    if len == 0 {
        return Ok(VmStackValue::Tuple(Vec::new()));
    }

    let mut values = vec![VmStackValue::Null; len];
    if len == 1 {
        values[0] = deserialize_cell_exact(slice.load_reference()?)?;
        return Ok(VmStackValue::Tuple(values));
    }

    let mut index = len - 1;
    let mut head = slice.load_reference()?;
    let mut tail = slice.load_reference()?;
    values[index] = deserialize_cell_exact(tail)?;

    while index > 1 {
        index -= 1;
        let mut node = Slice::new(head);
        head = node.load_reference()?;
        tail = node.load_reference()?;
        if !node.is_empty() {
            bail!("Trailing data after VM tuple pair");
        }
        values[index] = deserialize_cell_exact(tail)?;
    }
    values[0] = deserialize_cell_exact(head)?;
    Ok(VmStackValue::Tuple(values))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use num_bigint::BigInt;

    use crate::tvm::{
        Cell, Slice, TvmStack, TvmStackEntry, VmSlice, VmStack, VmStackValue,
        boc::{deserialize_boc, serialize_boc},
    };

    #[test]
    fn empty_stack_encodes_only_depth() {
        let stack = VmStack::default();
        let cell = stack.to_cell().unwrap();
        assert_eq!(cell.bit_len(), 24);
        assert_eq!(cell.references(), []);

        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_uint_custom::<u32>(24).unwrap(), 0);
        assert!(slice.is_empty());
        assert_eq!(VmStack::from_cell(stack.to_cell().unwrap()).unwrap(), stack);
    }

    #[test]
    fn encodes_expected_scalar_tags() {
        let stack = VmStack::new(vec![VmStackValue::Null, VmStackValue::int(-42)]);
        let cell = stack.to_cell().unwrap();
        let mut slice = Slice::new(cell);
        assert_eq!(slice.load_uint_custom::<u32>(24).unwrap(), 2);
        let _rest = slice.load_reference().unwrap();
        assert_eq!(slice.load_u8().unwrap(), 1);
        assert_eq!(slice.load_big_int(64).unwrap(), BigInt::from(-42));
    }

    #[test]
    fn round_trips_all_supported_values() {
        let mut code = crate::tvm::Builder::new();
        code.store_u32(0x1234_5678).unwrap();
        let code = code.build().unwrap();

        let tuple = VmStackValue::Tuple(vec![
            VmStackValue::int(1),
            VmStackValue::Null,
            VmStackValue::Tuple(vec![
                VmStackValue::int(-1),
                VmStackValue::Cell(code.clone()),
            ]),
        ]);
        let stack = VmStack::new(vec![
            VmStackValue::Null,
            VmStackValue::Nan,
            VmStackValue::int(0),
            VmStackValue::int(i64::MAX),
            VmStackValue::int(i64::MIN),
            VmStackValue::Int(
                BigInt::parse_bytes(b"1234567890123456789012345678901234567890", 10).unwrap(),
            ),
            VmStackValue::Cell(code.clone()),
            VmStackValue::Slice(VmSlice::new(code.clone(), 4, 20, 0, 0).unwrap()),
            VmStackValue::Builder(code),
            tuple,
        ]);

        let encoded = stack.to_boc().unwrap();
        let decoded = VmStack::from_boc(&encoded).unwrap();
        assert_eq!(decoded, stack);
        assert_eq!(decoded.to_boc().unwrap(), encoded);
    }

    #[test]
    fn preserves_slice_ranges() {
        let mut builder = crate::tvm::Builder::new();
        builder.store_u32(0xdead_beef).unwrap();
        let cell = builder.build().unwrap();
        let slice = VmSlice::new(cell, 3, 29, 0, 0).unwrap();
        let stack = VmStack::new(vec![VmStackValue::Slice(slice)]);

        let decoded = VmStack::from_boc(&stack.to_boc().unwrap()).unwrap();
        let VmStackValue::Slice(decoded_slice) = &decoded.values()[0] else {
            panic!("expected slice");
        };
        assert_eq!(decoded_slice.start_bits(), 3);
        assert_eq!(decoded_slice.end_bits(), 29);
        assert_eq!(decoded_slice.start_refs(), 0);
        assert_eq!(decoded_slice.end_refs(), 0);
    }

    #[test]
    fn rejects_partial_slice_conversion_to_high_level_stack() {
        let mut builder = crate::tvm::Builder::new();
        builder.store_u32(0xdead_beef).unwrap();
        let cell = builder.build().unwrap();
        let stack = VmStack::new(vec![VmStackValue::Slice(
            VmSlice::new(cell, 1, 31, 0, 0).unwrap(),
        )]);

        TvmStack::try_from(&stack).unwrap_err();
    }

    #[test]
    fn converts_full_range_high_level_stack() {
        let mut builder = crate::tvm::Builder::new();
        builder.store_u32(0xdead_beef).unwrap();
        let cell = builder.build().unwrap();
        let high = TvmStack::new(vec![
            TvmStackEntry::Null,
            TvmStackEntry::int(7),
            TvmStackEntry::Cell(cell.clone()),
            TvmStackEntry::Slice(cell),
        ]);

        let standard = VmStack::try_from(&high).unwrap();
        assert_eq!(standard.len(), 4);
        assert_eq!(TvmStack::try_from(&standard).unwrap(), high);
    }

    #[test]
    fn rejects_invalid_slice_ranges() {
        let cell = Arc::new(Cell::with_data(vec![0xAA], 8).unwrap());
        VmSlice::new(cell.clone(), 9, 8, 0, 0).unwrap_err();
        VmSlice::new(cell.clone(), 0, 9, 0, 0).unwrap_err();
        VmSlice::new(cell, 0, 8, 0, 1).unwrap_err();
    }

    #[test]
    fn rejects_continuation_tag() {
        let mut builder = crate::tvm::Builder::new();
        builder.store_uint_custom::<u32>(1, 24).unwrap();
        builder
            .store_ref(crate::tvm::Builder::new().build().unwrap())
            .unwrap();
        builder.store_u8(6).unwrap();
        let cell = builder.build().unwrap();

        VmStack::from_cell(cell).unwrap_err();
    }

    #[test]
    fn boc_round_trip_avoids_checksum() {
        let stack = VmStack::new(vec![VmStackValue::int(1)]);
        let boc = stack.to_boc().unwrap();
        assert_eq!(deserialize_boc(&boc).unwrap(), stack.to_cell().unwrap());
        assert_eq!(
            serialize_boc(&stack.to_cell().unwrap(), false).unwrap(),
            boc
        );
    }

    #[test]
    fn fixture_stack_is_not_standard_stack_format() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/tvm/stack.json")).unwrap();
        let vector = &fixture["vectors"][0];
        let boc = hex::decode(vector["input_stack_boc_hex"].as_str().unwrap()).unwrap();
        VmStack::from_boc(&boc).unwrap_err();
    }
}
