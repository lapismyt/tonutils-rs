//! TON HashmapE dictionary support.
//!
//! TON dictionaries are canonical Patricia trees over fixed-width bitstring
//! keys. `HashmapE n X` stores either `hme_empty$0` or `hme_root$1` followed by a
//! reference to a `Hashmap n X` edge.

use crate::tvm::address::Address;
use crate::tvm::builder::Builder;
use crate::tvm::cell::Cell;
use crate::tvm::slice::Slice;
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Fixed-width MSB-first dictionary key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BitKey {
    data: Vec<u8>,
    bit_len: usize,
}

impl BitKey {
    /// Creates a canonical key and verifies that unused final-byte bits are zero.
    pub fn new(data: Vec<u8>, bit_len: usize) -> Result<Self> {
        let required_bytes = bits_to_bytes(bit_len);
        if data.len() != required_bytes {
            bail!(
                "BitKey data length {} does not match {} bits",
                data.len(),
                bit_len
            );
        }
        if bit_len == 0 {
            return Ok(Self { data, bit_len });
        }
        let unused = data.len() * 8 - bit_len;
        if unused > 0 {
            let mask = (1u8 << unused) - 1;
            if data[data.len() - 1] & mask != 0 {
                bail!("BitKey unused final-byte bits must be zero");
            }
        }
        Ok(Self { data, bit_len })
    }

    /// Creates a canonical key by clearing unused final-byte bits.
    pub fn from_bits(mut data: Vec<u8>, bit_len: usize) -> Result<Self> {
        let required_bytes = bits_to_bytes(bit_len);
        if data.len() < required_bytes {
            bail!(
                "Insufficient BitKey data: {} bytes for {} bits",
                data.len(),
                bit_len
            );
        }
        data.truncate(required_bytes);
        clear_unused_bits(&mut data, bit_len);
        Ok(Self { data, bit_len })
    }

    /// Creates a fixed-width key from a `u64`.
    pub fn from_u64(value: u64, bit_len: usize) -> Result<Self> {
        if bit_len > 64 {
            bail!("u64 key cannot represent {} bits", bit_len);
        }
        if bit_len < 64 && value >= (1u64 << bit_len) {
            bail!("Integer key does not fit in {} bits", bit_len);
        }

        let mut data = vec![0u8; bits_to_bytes(bit_len)];
        for bit_index in 0..bit_len {
            let source_shift = bit_len - bit_index - 1;
            let bit = ((value >> source_shift) & 1) != 0;
            set_bit(&mut data, bit_index, bit);
        }
        Ok(Self { data, bit_len })
    }

    /// Returns the key as `u64` when it fits.
    pub fn to_u64(&self) -> Result<u64> {
        if self.bit_len > 64 {
            bail!("BitKey with {} bits does not fit u64", self.bit_len);
        }
        let mut value = 0u64;
        for index in 0..self.bit_len {
            value <<= 1;
            if self.bit(index)? {
                value |= 1;
            }
        }
        Ok(value)
    }

    /// Serializes a standard internal address key (`addr_std`) as 267 bits.
    pub fn from_address(address: &Address) -> Result<Self> {
        let mut builder = Builder::new();
        builder.store_address(Some(address))?;
        let cell = builder.build()?;
        Self::from_bits(cell.data().to_vec(), cell.bit_len())
    }

    /// Returns key bytes with MSB-first bit packing.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns key length in bits.
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    /// Returns the bit at `index`.
    pub fn bit(&self, index: usize) -> Result<bool> {
        if index >= self.bit_len {
            bail!(
                "Bit index {} out of bounds for {} bits",
                index,
                self.bit_len
            );
        }
        Ok(get_bit(&self.data, index))
    }

    /// Returns a prefix key of `bit_len` bits.
    pub fn prefix(&self, bit_len: usize) -> Result<Self> {
        if bit_len > self.bit_len {
            bail!("Prefix length exceeds key length");
        }
        let mut data = vec![0u8; bits_to_bytes(bit_len)];
        for index in 0..bit_len {
            set_bit(&mut data, index, self.bit(index)?);
        }
        Ok(Self { data, bit_len })
    }
}

/// Generic TON `HashmapE n X` dictionary.
#[derive(Debug, Clone)]
pub struct HashmapE<V> {
    key_bits: usize,
    map: BTreeMap<BitKey, V>,
}

impl<V> HashmapE<V> {
    /// Creates an empty dictionary with fixed key width.
    pub fn new(key_bits: usize) -> Self {
        Self {
            key_bits,
            map: BTreeMap::new(),
        }
    }

    /// Returns the fixed key width in bits.
    pub fn key_bits(&self) -> usize {
        self.key_bits
    }

    /// Inserts a value under a fixed-width bit key.
    pub fn insert_bit_key(&mut self, key: BitKey, value: V) -> Result<Option<V>> {
        if key.bit_len() != self.key_bits {
            bail!(
                "Dictionary key length {} does not match {}",
                key.bit_len(),
                self.key_bits
            );
        }
        Ok(self.map.insert(key, value))
    }

    /// Gets a value by fixed-width bit key.
    pub fn get_bit_key(&self, key: &BitKey) -> Result<Option<&V>> {
        if key.bit_len() != self.key_bits {
            bail!(
                "Dictionary key length {} does not match {}",
                key.bit_len(),
                self.key_bits
            );
        }
        Ok(self.map.get(key))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns true when the dictionary has no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates entries in canonical key order.
    pub fn iter(&self) -> impl Iterator<Item = (&BitKey, &V)> {
        self.map.iter()
    }
}

/// Dictionary key type preserved for compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DictKey {
    /// Integer key.
    Int(u64),
    /// Binary key (as bit string).
    Bits(Vec<u8>, usize),
    /// Standard address key (267 bits).
    Address(Address),
}

/// Dictionary value type preserved for compatibility.
#[derive(Debug, Clone)]
pub enum DictValue {
    /// Cell value.
    Cell(Arc<Cell>),
    /// Slice value.
    Slice(Slice),
    /// Integer value.
    Uint(u64, usize),
    /// Signed integer value.
    Int(i64, usize),
    /// Coins value.
    Coins(u128),
    /// Address value.
    Address(Address),
}

/// Compatibility dictionary wrapper backed by `HashmapE<DictValue>`.
#[derive(Debug, Clone)]
pub struct Dict {
    inner: HashmapE<DictValue>,
    int_keys: BTreeMap<u64, BitKey>,
}

impl Dict {
    /// Creates a new dictionary with the specified key size.
    pub fn new(key_size: usize) -> Self {
        Self {
            inner: HashmapE::new(key_size),
            int_keys: BTreeMap::new(),
        }
    }

    /// Sets a value for an integer key.
    pub fn set_int_key(&mut self, key: u64, value: DictValue) -> Result<&mut Self> {
        let bit_key = BitKey::from_u64(key, self.key_size())?;
        self.inner.insert_bit_key(bit_key.clone(), value)?;
        self.int_keys.insert(key, bit_key);
        Ok(self)
    }

    /// Sets a value for a key.
    pub fn set(&mut self, key: DictKey, value: DictValue) -> Result<&mut Self> {
        let bit_key = self.dict_key_to_bit_key(&key)?;
        if let DictKey::Int(int_key) = key {
            self.int_keys.insert(int_key, bit_key.clone());
        }
        self.inner.insert_bit_key(bit_key, value)?;
        Ok(self)
    }

    /// Gets a value by integer key.
    pub fn get_int_key(&self, key: u64) -> Option<&DictValue> {
        self.int_keys
            .get(&key)
            .and_then(|bit_key| self.inner.map.get(bit_key))
    }

    /// Gets a value by key.
    pub fn get(&self, key: &DictKey) -> Result<Option<&DictValue>> {
        let bit_key = self.dict_key_to_bit_key(key)?;
        Ok(self.inner.map.get(&bit_key))
    }

    /// Returns the number of entries in the dictionary.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the key size in bits.
    pub fn key_size(&self) -> usize {
        self.inner.key_bits()
    }

    /// Serializes the dictionary to a `Hashmap n X` root cell, or `None` if empty.
    pub fn serialize(&self) -> Result<Option<Arc<Cell>>> {
        serialize_hashmap_root(&self.inner, store_dict_value)
    }

    /// Deserializes a dictionary by storing each raw value node as `DictValue::Slice`.
    pub fn deserialize(cell: &Arc<Cell>, key_size: usize) -> Result<Self> {
        let inner = deserialize_hashmap_root(cell, key_size, |slice| {
            Ok(DictValue::Slice(slice.clone_from_current()))
        })?;
        Ok(Self {
            inner,
            int_keys: BTreeMap::new(),
        })
    }

    /// Creates an iterator over integer entries inserted through integer APIs.
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &DictValue)> {
        self.int_keys.iter().filter_map(|(int_key, bit_key)| {
            self.inner.map.get(bit_key).map(|value| (int_key, value))
        })
    }

    /// Iterates all entries by bit key.
    pub fn iter_bit_keys(&self) -> impl Iterator<Item = (&BitKey, &DictValue)> {
        self.inner.iter()
    }

    fn dict_key_to_bit_key(&self, key: &DictKey) -> Result<BitKey> {
        match key {
            DictKey::Int(key) => BitKey::from_u64(*key, self.key_size()),
            DictKey::Bits(data, bit_len) => {
                if *bit_len != self.key_size() {
                    bail!("Key bit length must match dictionary key size");
                }
                BitKey::from_bits(data.clone(), *bit_len)
            }
            DictKey::Address(address) => {
                if self.key_size() != 267 {
                    bail!("Address keys require key_size of 267 bits");
                }
                BitKey::from_address(address)
            }
        }
    }
}

impl Default for Dict {
    fn default() -> Self {
        Self::new(256)
    }
}

impl Builder {
    /// Stores a `HashmapE n X` using the supplied value encoder.
    pub fn store_hashmap_e_with<V, F>(
        &mut self,
        dict: &HashmapE<V>,
        store_value: F,
    ) -> Result<&mut Self>
    where
        F: Fn(&mut Builder, &V) -> Result<()>,
    {
        if let Some(root) = serialize_hashmap_root(dict, store_value)? {
            self.store_bit(true)?;
            self.store_ref(root)?;
        } else {
            self.store_bit(false)?;
        }
        Ok(self)
    }

    /// Stores a compatibility dictionary as `HashmapE`.
    pub fn store_dictionary(&mut self, dict: Option<&Dict>) -> Result<&mut Self> {
        match dict {
            Some(dict) => self.store_hashmap_e_with(&dict.inner, store_dict_value),
            None => {
                self.store_bit(false)?;
                Ok(self)
            }
        }
    }
}

impl Slice {
    /// Loads a `HashmapE n X` using the supplied value decoder.
    pub fn load_hashmap_e_with<V, F>(
        &mut self,
        key_bits: usize,
        load_value: F,
    ) -> Result<HashmapE<V>>
    where
        F: Fn(&mut Slice) -> Result<V>,
    {
        if !self.load_bit()? {
            return Ok(HashmapE::new(key_bits));
        }
        let root = self.load_reference()?;
        deserialize_hashmap_root(&root, key_bits, load_value)
    }

    /// Loads a compatibility dictionary from `HashmapE`.
    pub fn load_dict(&mut self, key_size: usize) -> Result<Option<Dict>> {
        if !self.load_bit()? {
            return Ok(None);
        }
        let root = self.load_reference()?;
        Ok(Some(Dict::deserialize(&root, key_size)?))
    }
}

fn store_dict_value(builder: &mut Builder, value: &DictValue) -> Result<()> {
    match value {
        DictValue::Cell(cell) => {
            builder.store_ref(cell.clone())?;
        }
        DictValue::Slice(slice) => {
            builder.store_slice(slice)?;
        }
        DictValue::Uint(value, bits) => {
            builder.store_uint(*value, *bits)?;
        }
        DictValue::Int(value, bits) => {
            builder.store_int(*value, *bits)?;
        }
        DictValue::Coins(value) => {
            builder.store_coins(*value)?;
        }
        DictValue::Address(address) => {
            builder.store_address(Some(address))?;
        }
    }
    Ok(())
}

fn serialize_hashmap_root<V, F>(dict: &HashmapE<V>, store_value: F) -> Result<Option<Arc<Cell>>>
where
    F: Fn(&mut Builder, &V) -> Result<()>,
{
    if dict.is_empty() {
        return Ok(None);
    }
    let entries: Vec<_> = dict.iter().collect();
    Ok(Some(build_edge(
        &entries,
        0,
        dict.key_bits(),
        &store_value,
    )?))
}

fn deserialize_hashmap_root<V, F>(
    cell: &Arc<Cell>,
    key_bits: usize,
    load_value: F,
) -> Result<HashmapE<V>>
where
    F: Fn(&mut Slice) -> Result<V>,
{
    let mut slice = Slice::new(cell.clone());
    let mut dict = HashmapE::new(key_bits);
    parse_edge(
        &mut slice,
        key_bits,
        &mut Vec::new(),
        &mut dict,
        &load_value,
    )?;
    Ok(dict)
}

fn build_edge<V, F>(
    entries: &[(&BitKey, &V)],
    depth: usize,
    remaining: usize,
    store_value: &F,
) -> Result<Arc<Cell>>
where
    F: Fn(&mut Builder, &V) -> Result<()>,
{
    if entries.is_empty() {
        bail!("Cannot build empty Hashmap edge");
    }

    let label_len = common_prefix_len(entries, depth, remaining)?;
    let label_bits = collect_key_bits(entries[0].0, depth, label_len)?;
    let node_remaining = remaining - label_len;

    let mut builder = Builder::new();
    store_label(&mut builder, &label_bits, remaining)?;

    if node_remaining == 0 {
        if entries.len() != 1 {
            bail!("Duplicate dictionary key");
        }
        store_value(&mut builder, entries[0].1)?;
    } else {
        let split_depth = depth + label_len;
        let split = entries
            .iter()
            .position(|(key, _)| key.bit(split_depth).unwrap_or(false))
            .unwrap_or(entries.len());
        if split == 0 || split == entries.len() {
            bail!("Invalid dictionary fork without both branches");
        }
        let left = build_edge(
            &entries[..split],
            split_depth + 1,
            node_remaining - 1,
            store_value,
        )?;
        let right = build_edge(
            &entries[split..],
            split_depth + 1,
            node_remaining - 1,
            store_value,
        )?;
        builder.store_ref(left)?;
        builder.store_ref(right)?;
    }

    builder.build()
}

fn parse_edge<V, F>(
    slice: &mut Slice,
    remaining: usize,
    prefix: &mut Vec<bool>,
    dict: &mut HashmapE<V>,
    load_value: &F,
) -> Result<()>
where
    F: Fn(&mut Slice) -> Result<V>,
{
    let label = load_label(slice, remaining)?;
    let label_len = label.len();
    prefix.extend(label);
    let node_remaining = remaining - label_len;

    if node_remaining == 0 {
        let key = bit_vec_to_key(prefix, dict.key_bits())?;
        let value = load_value(slice)?;
        if dict.insert_bit_key(key, value)?.is_some() {
            bail!("Duplicate dictionary key");
        }
    } else {
        let left = slice.load_reference()?;
        let right = slice.load_reference()?;

        prefix.push(false);
        let mut left_slice = Slice::new(left);
        parse_edge(
            &mut left_slice,
            node_remaining - 1,
            prefix,
            dict,
            load_value,
        )?;
        prefix.pop();

        prefix.push(true);
        let mut right_slice = Slice::new(right);
        parse_edge(
            &mut right_slice,
            node_remaining - 1,
            prefix,
            dict,
            load_value,
        )?;
        prefix.pop();
    }

    prefix.truncate(prefix.len() - label_len);
    Ok(())
}

fn store_label(builder: &mut Builder, bits: &[bool], max_len: usize) -> Result<()> {
    let encoded = canonical_label(bits, max_len)?;
    for bit in encoded {
        builder.store_bit(bit)?;
    }
    Ok(())
}

fn load_label(slice: &mut Slice, max_len: usize) -> Result<Vec<bool>> {
    let first = slice.load_bit()?;
    if !first {
        let len = load_unary(slice, max_len)?;
        let mut bits = Vec::with_capacity(len);
        for _ in 0..len {
            bits.push(slice.load_bit()?);
        }
        return Ok(bits);
    }

    let second = slice.load_bit()?;
    let width = label_len_width(max_len);
    if !second {
        let len = load_label_len(slice, width)?;
        if len > max_len {
            bail!("Long Hashmap label length exceeds remaining key bits");
        }
        let mut bits = Vec::with_capacity(len);
        for _ in 0..len {
            bits.push(slice.load_bit()?);
        }
        Ok(bits)
    } else {
        let value = slice.load_bit()?;
        let len = load_label_len(slice, width)?;
        if len > max_len {
            bail!("Same Hashmap label length exceeds remaining key bits");
        }
        Ok(vec![value; len])
    }
}

fn canonical_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    if bits.len() > max_len {
        bail!("Hashmap label length exceeds remaining key bits");
    }

    let mut candidates = vec![encode_short_label(bits), encode_long_label(bits, max_len)?];
    if bits.iter().all(|bit| *bit == false) || bits.iter().all(|bit| *bit == true) {
        candidates.push(encode_same_label(bits, max_len)?);
    }
    candidates.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
    Ok(candidates.remove(0))
}

fn encode_short_label(bits: &[bool]) -> Vec<bool> {
    let mut encoded = Vec::with_capacity(2 + bits.len() * 2);
    encoded.push(false);
    encoded.extend(std::iter::repeat(true).take(bits.len()));
    encoded.push(false);
    encoded.extend_from_slice(bits);
    encoded
}

fn encode_long_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    let width = label_len_width(max_len);
    let mut encoded = Vec::with_capacity(2 + width + bits.len());
    encoded.push(true);
    encoded.push(false);
    push_uint_bits(&mut encoded, bits.len(), width)?;
    encoded.extend_from_slice(bits);
    Ok(encoded)
}

fn encode_same_label(bits: &[bool], max_len: usize) -> Result<Vec<bool>> {
    let width = label_len_width(max_len);
    let mut encoded = Vec::with_capacity(3 + width);
    encoded.push(true);
    encoded.push(true);
    encoded.push(bits.first().copied().unwrap_or(false));
    push_uint_bits(&mut encoded, bits.len(), width)?;
    Ok(encoded)
}

fn load_unary(slice: &mut Slice, max_len: usize) -> Result<usize> {
    let mut len = 0usize;
    loop {
        let bit = slice.load_bit()?;
        if !bit {
            return Ok(len);
        }
        len += 1;
        if len > max_len {
            bail!("Short Hashmap label length exceeds remaining key bits");
        }
    }
}

fn load_label_len(slice: &mut Slice, width: usize) -> Result<usize> {
    let mut value = 0usize;
    for _ in 0..width {
        value <<= 1;
        if slice.load_bit()? {
            value |= 1;
        }
    }
    Ok(value)
}

fn label_len_width(max_len: usize) -> usize {
    let mut width = 0usize;
    let mut value = max_len;
    while value > 0 {
        width += 1;
        value >>= 1;
    }
    width
}

fn push_uint_bits(bits: &mut Vec<bool>, value: usize, width: usize) -> Result<()> {
    if width < usize::BITS as usize && value >= (1usize << width) {
        bail!("Value {} does not fit in {} bits", value, width);
    }
    for shift in (0..width).rev() {
        bits.push(((value >> shift) & 1) != 0);
    }
    Ok(())
}

fn common_prefix_len<V>(entries: &[(&BitKey, &V)], depth: usize, max_len: usize) -> Result<usize> {
    let first = entries[0].0;
    let mut len = 0usize;
    'outer: while len < max_len {
        let bit = first.bit(depth + len)?;
        for (key, _) in &entries[1..] {
            if key.bit(depth + len)? != bit {
                break 'outer;
            }
        }
        len += 1;
    }
    Ok(len)
}

fn collect_key_bits(key: &BitKey, offset: usize, len: usize) -> Result<Vec<bool>> {
    let mut bits = Vec::with_capacity(len);
    for index in offset..offset + len {
        bits.push(key.bit(index)?);
    }
    Ok(bits)
}

fn bit_vec_to_key(bits: &[bool], bit_len: usize) -> Result<BitKey> {
    if bits.len() != bit_len {
        bail!(
            "Decoded key length {} does not match {}",
            bits.len(),
            bit_len
        );
    }
    let mut data = vec![0u8; bits_to_bytes(bit_len)];
    for (index, bit) in bits.iter().copied().enumerate() {
        set_bit(&mut data, index, bit);
    }
    BitKey::new(data, bit_len)
}

fn get_bit(data: &[u8], index: usize) -> bool {
    (data[index / 8] >> (7 - (index % 8))) & 1 == 1
}

fn set_bit(data: &mut [u8], index: usize, bit: bool) {
    if bit {
        data[index / 8] |= 1 << (7 - (index % 8));
    }
}

fn bits_to_bytes(bits: usize) -> usize {
    (bits + 7) / 8
}

fn clear_unused_bits(data: &mut [u8], bit_len: usize) {
    if data.is_empty() {
        return;
    }
    let unused = data.len() * 8 - bit_len;
    if unused > 0 {
        data[data.len() - 1] &= !((1u8 << unused) - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_key_canonicalizes_and_orders() {
        assert!(BitKey::new(vec![0b1010_0001], 4).is_err());
        let key = BitKey::from_bits(vec![0b1010_1111], 4).unwrap();
        assert_eq!(key.data(), &[0b1010_0000]);
        assert!(key.bit(0).unwrap());
        assert!(!key.bit(1).unwrap());
        assert_eq!(key.prefix(3).unwrap().data(), &[0b1010_0000]);

        let low = BitKey::from_bits(vec![0b0100_0000], 2).unwrap();
        let high = BitKey::from_bits(vec![0b1000_0000], 2).unwrap();
        assert!(low < high);
    }

    #[test]
    fn labels_choose_canonical_encoding() {
        assert_eq!(canonical_label(&[], 0).unwrap(), vec![false, false]);
        assert_eq!(
            canonical_label(&[true, false], 3).unwrap(),
            vec![false, true, true, false, true, false]
        );
        assert_eq!(
            canonical_label(&[false, false, false, false], 8).unwrap(),
            vec![true, true, false, false, true, false, false]
        );
    }

    #[test]
    fn hashmap_e_roundtrips_uint_values() {
        let mut dict = HashmapE::new(8);
        dict.insert_bit_key(BitKey::from_u64(0b1010_0000, 8).unwrap(), 10u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(0b1010_1111, 8).unwrap(), 20u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(0b1111_0000, 8).unwrap(), 30u64)
            .unwrap();

        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_uint(*value, 16)?;
                Ok(())
            })
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(8, |slice| slice.load_uint(16))
            .unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(0b1010_1111, 8).unwrap())
                .unwrap(),
            Some(&20)
        );
    }

    #[test]
    fn hashmap_e_roundtrips_empty_and_wide_keys() {
        let empty: HashmapE<u64> = HashmapE::new(256);
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&empty, |builder, value| {
                builder.store_uint(*value, 1)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        assert!(
            slice
                .load_hashmap_e_with(256, |slice| slice.load_uint(1))
                .unwrap()
                .is_empty()
        );

        let mut dict = HashmapE::new(267);
        let key = BitKey::from_bits(vec![0xAA; 34], 267).unwrap();
        dict.insert_bit_key(key.clone(), 7u64).unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_uint(*value, 4)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(267, |slice| slice.load_uint(4))
            .unwrap();
        assert_eq!(decoded.get_bit_key(&key).unwrap(), Some(&7));
    }

    #[test]
    fn hashmap_e_roundtrips_callback_value_codecs() {
        let mut coins = HashmapE::new(4);
        coins
            .insert_bit_key(BitKey::from_u64(1, 4).unwrap(), 1_000_000_000u128)
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&coins, |builder, value| {
                builder.store_coins(*value)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| slice.load_coins())
            .unwrap();
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(1, 4).unwrap())
                .unwrap(),
            Some(&1_000_000_000)
        );

        let address = Address::new(-1, [0x44; 32]);
        let mut addresses = HashmapE::new(4);
        addresses
            .insert_bit_key(BitKey::from_u64(2, 4).unwrap(), address.clone())
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&addresses, |builder, value| {
                builder.store_address(Some(value))?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| {
                let data = slice.load_bits(267)?;
                let mut builder = Builder::new();
                builder.store_bits(&data, 267)?;
                Ok(builder.build()?)
            })
            .unwrap();
        let cell = decoded
            .get_bit_key(&BitKey::from_u64(2, 4).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(cell.bit_len(), 267);

        let mut raw_builder = Builder::new();
        raw_builder.store_bits(&[0b1010_0000], 4).unwrap();
        let raw_cell = raw_builder.build().unwrap();
        let mut cells = HashmapE::new(4);
        cells
            .insert_bit_key(BitKey::from_u64(3, 4).unwrap(), raw_cell.clone())
            .unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&cells, |builder, value| {
                builder.store_ref(value.clone())?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(4, |slice| slice.load_reference())
            .unwrap();
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(3, 4).unwrap())
                .unwrap()
                .unwrap()
                .hash(),
            raw_cell.hash()
        );
    }

    #[test]
    fn dict_address_keys_are_not_truncated() {
        let mut dict = Dict::new(267);
        let address = Address::new(0, [0x11; 32]);
        dict.set(DictKey::Address(address.clone()), DictValue::Coins(1))
            .unwrap();
        assert!(dict.get(&DictKey::Address(address)).unwrap().is_some());
    }

    #[test]
    fn malformed_labels_and_missing_refs_fail() {
        let mut builder = Builder::new();
        builder.store_bit(false).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(true).unwrap();
        builder.store_bit(false).unwrap();
        let mut slice = builder.to_slice().unwrap();
        assert!(load_label(&mut slice, 1).is_err());

        let mut dict = HashmapE::new(2);
        dict.insert_bit_key(BitKey::from_u64(0, 2).unwrap(), 1u64)
            .unwrap();
        dict.insert_bit_key(BitKey::from_u64(2, 2).unwrap(), 2u64)
            .unwrap();
        let root = serialize_hashmap_root(&dict, |builder, value| {
            builder.store_uint(*value, 2)?;
            Ok(())
        })
        .unwrap()
        .unwrap();
        let mut broken = Builder::new();
        broken.store_bits(root.data(), root.bit_len()).unwrap();
        let broken = broken.build().unwrap();
        assert!(deserialize_hashmap_root(&broken, 2, |slice| slice.load_uint(2)).is_err());
    }
}
