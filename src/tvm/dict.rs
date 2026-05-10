//! TON HashmapE and HashmapAugE dictionary support.
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// A decoded `HashmapAug` leaf entry with its per-leaf augmentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashmapAugLeaf<V, E> {
    /// Fixed-width dictionary key.
    pub key: BitKey,
    /// Value stored at the leaf.
    pub value: V,
    /// Augmentation stored by `ahmn_leaf`.
    pub extra: E,
}

/// A decoded `HashmapAug` fork augmentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashmapAugFork<E> {
    /// Prefix bits before the fork branch bit.
    pub prefix: BitKey,
    /// Augmentation stored by `ahmn_fork`.
    pub extra: E,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HashmapAugNode<V, E> {
    Leaf {
        label: Vec<bool>,
        extra: E,
        value: V,
    },
    Fork {
        label: Vec<bool>,
        left: Box<HashmapAugNode<V, E>>,
        right: Box<HashmapAugNode<V, E>>,
        extra: E,
    },
}

/// Generic non-empty TON `HashmapAug n X Y` dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashmapAug<V, E> {
    key_bits: usize,
    root: HashmapAugNode<V, E>,
    leaves: BTreeMap<BitKey, (V, E)>,
    forks: Vec<HashmapAugFork<E>>,
}

impl<V, E> HashmapAug<V, E> {
    /// Creates a canonical augmented dictionary from key-ordered leaves.
    ///
    /// The supplied `fork_extra` is written to every generated fork node. TON
    /// aggregation semantics are schema-specific, so callers that need
    /// meaningful fork augmentations should compute the value before calling.
    pub fn from_entries(
        key_bits: usize,
        entries: Vec<HashmapAugLeaf<V, E>>,
        fork_extra: E,
    ) -> Result<Self>
    where
        V: Clone,
        E: Clone,
    {
        if entries.is_empty() {
            bail!("HashmapAug cannot be empty");
        }

        let mut leaves = BTreeMap::new();
        for entry in entries {
            if entry.key.bit_len() != key_bits {
                bail!(
                    "Dictionary key length {} does not match {}",
                    entry.key.bit_len(),
                    key_bits
                );
            }
            if leaves
                .insert(entry.key, (entry.value, entry.extra))
                .is_some()
            {
                bail!("Duplicate dictionary key");
            }
        }

        let ordered: Vec<_> = leaves
            .iter()
            .map(|(key, (value, extra))| (key, value, extra))
            .collect();
        let mut forks = Vec::new();
        let root = build_aug_node(&ordered, 0, key_bits, &fork_extra, &mut forks)?;
        Ok(Self {
            key_bits,
            root,
            leaves,
            forks,
        })
    }

    /// Returns the fixed key width in bits.
    pub fn key_bits(&self) -> usize {
        self.key_bits
    }

    /// Number of leaves.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Returns false because `HashmapAug` has no empty constructor.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Iterates leaves in canonical key order.
    pub fn iter(&self) -> impl Iterator<Item = (&BitKey, &V, &E)> {
        self.leaves
            .iter()
            .map(|(key, (value, extra))| (key, value, extra))
    }

    /// Gets a leaf value and augmentation by fixed-width bit key.
    pub fn get_bit_key(&self, key: &BitKey) -> Result<Option<(&V, &E)>> {
        if key.bit_len() != self.key_bits {
            bail!(
                "Dictionary key length {} does not match {}",
                key.bit_len(),
                self.key_bits
            );
        }
        Ok(self.leaves.get(key).map(|(value, extra)| (value, extra)))
    }

    /// Returns decoded fork augmentations in depth-first order.
    pub fn fork_extras(&self) -> &[HashmapAugFork<E>] {
        &self.forks
    }
}

/// Generic TON `HashmapAugE n X Y` dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashmapAugE<V, E> {
    key_bits: usize,
    root: Option<HashmapAug<V, E>>,
    extra: E,
}

impl<V, E> HashmapAugE<V, E> {
    /// Creates an empty augmented dictionary with the top-level extra value.
    pub fn empty(key_bits: usize, extra: E) -> Self {
        Self {
            key_bits,
            root: None,
            extra,
        }
    }

    /// Creates a non-empty augmented dictionary with the top-level extra value.
    pub fn with_root(key_bits: usize, root: HashmapAug<V, E>, extra: E) -> Result<Self> {
        if root.key_bits() != key_bits {
            bail!(
                "Dictionary key length {} does not match {}",
                root.key_bits(),
                key_bits
            );
        }
        Ok(Self {
            key_bits,
            root: Some(root),
            extra,
        })
    }

    /// Returns the fixed key width in bits.
    pub fn key_bits(&self) -> usize {
        self.key_bits
    }

    /// Returns the top-level `HashmapAugE` extra.
    pub fn extra(&self) -> &E {
        &self.extra
    }

    /// Returns the non-empty root, when present.
    pub fn root(&self) -> Option<&HashmapAug<V, E>> {
        self.root.as_ref()
    }

    /// Number of leaves.
    pub fn len(&self) -> usize {
        self.root.as_ref().map(HashmapAug::len).unwrap_or(0)
    }

    /// Returns true when the dictionary has no root.
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Iterates leaves in canonical key order.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&BitKey, &V, &E)> + '_> {
        match &self.root {
            Some(root) => Box::new(root.iter()),
            None => Box::new(std::iter::empty()),
        }
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

    /// Stores a non-empty `HashmapAug n X Y` using supplied value and extra encoders.
    pub fn store_hashmap_aug_with<V, E, FV, FE>(
        &mut self,
        dict: &HashmapAug<V, E>,
        store_value: FV,
        store_extra: FE,
    ) -> Result<&mut Self>
    where
        FV: Fn(&mut Builder, &V) -> Result<()>,
        FE: Fn(&mut Builder, &E) -> Result<()>,
    {
        store_aug_node(self, &dict.root, &store_value, &store_extra)?;
        Ok(self)
    }

    /// Stores a `HashmapAugE n X Y` using supplied value and extra encoders.
    pub fn store_hashmap_aug_e_with<V, E, FV, FE>(
        &mut self,
        dict: &HashmapAugE<V, E>,
        store_value: FV,
        store_extra: FE,
    ) -> Result<&mut Self>
    where
        FV: Fn(&mut Builder, &V) -> Result<()>,
        FE: Fn(&mut Builder, &E) -> Result<()>,
    {
        match dict.root() {
            Some(root) => {
                self.store_bit(true)?;
                let mut root_builder = Builder::new();
                root_builder.store_hashmap_aug_with(root, &store_value, &store_extra)?;
                self.store_ref(root_builder.build()?)?;
            }
            None => {
                self.store_bit(false)?;
            }
        }
        store_extra(self, dict.extra())?;
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

    /// Loads a non-empty `HashmapAug n X Y`.
    pub fn load_hashmap_aug_with<V, E, FV, FE>(
        &mut self,
        key_bits: usize,
        load_value: FV,
        load_extra: FE,
    ) -> Result<HashmapAug<V, E>>
    where
        V: Clone,
        E: Clone,
        FV: Fn(&mut Slice) -> Result<V>,
        FE: Fn(&mut Slice) -> Result<E>,
    {
        deserialize_hashmap_aug_from_slice(self, key_bits, load_value, load_extra)
    }

    /// Loads a `HashmapAugE n X Y`.
    pub fn load_hashmap_aug_e_with<V, E, FV, FE>(
        &mut self,
        key_bits: usize,
        load_value: FV,
        load_extra: FE,
    ) -> Result<HashmapAugE<V, E>>
    where
        V: Clone,
        E: Clone,
        FV: Fn(&mut Slice) -> Result<V>,
        FE: Fn(&mut Slice) -> Result<E>,
    {
        let has_root = self.load_bit()?;
        if has_root {
            let root_cell = self.load_reference()?;
            let mut root_slice = Slice::new(root_cell);
            let root = deserialize_hashmap_aug_from_slice(
                &mut root_slice,
                key_bits,
                &load_value,
                &load_extra,
            )?;
            ensure_aug_ref_consumed(&root_slice)?;
            let extra = load_extra(self)?;
            HashmapAugE::with_root(key_bits, root, extra)
        } else {
            let extra = load_extra(self)?;
            Ok(HashmapAugE::empty(key_bits, extra))
        }
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
            builder.store_big_uint(&num_bigint::BigUint::from(*value), *bits)?;
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

fn build_aug_node<V, E>(
    entries: &[(&BitKey, &V, &E)],
    depth: usize,
    remaining: usize,
    fork_extra: &E,
    forks: &mut Vec<HashmapAugFork<E>>,
) -> Result<HashmapAugNode<V, E>>
where
    V: Clone,
    E: Clone,
{
    if entries.is_empty() {
        bail!("Cannot build empty HashmapAug edge");
    }

    let label_len = common_aug_prefix_len(entries, depth, remaining)?;
    let label = collect_key_bits(entries[0].0, depth, label_len)?;
    let node_remaining = remaining - label_len;

    if node_remaining == 0 {
        if entries.len() != 1 {
            bail!("Duplicate dictionary key");
        }
        return Ok(HashmapAugNode::Leaf {
            label,
            extra: entries[0].2.clone(),
            value: entries[0].1.clone(),
        });
    }

    let split_depth = depth + label_len;
    let split = entries
        .iter()
        .position(|(key, _, _)| key.bit(split_depth).unwrap_or(false))
        .unwrap_or(entries.len());
    if split == 0 || split == entries.len() {
        bail!("Invalid augmented dictionary fork without both branches");
    }

    let mut prefix = Vec::with_capacity(split_depth);
    for index in 0..split_depth {
        prefix.push(entries[0].0.bit(index)?);
    }
    forks.push(HashmapAugFork {
        prefix: bit_vec_to_key(&prefix, split_depth)?,
        extra: fork_extra.clone(),
    });

    Ok(HashmapAugNode::Fork {
        label,
        left: Box::new(build_aug_node(
            &entries[..split],
            split_depth + 1,
            node_remaining - 1,
            fork_extra,
            forks,
        )?),
        right: Box::new(build_aug_node(
            &entries[split..],
            split_depth + 1,
            node_remaining - 1,
            fork_extra,
            forks,
        )?),
        extra: fork_extra.clone(),
    })
}

fn deserialize_hashmap_aug_from_slice<V, E, FV, FE>(
    slice: &mut Slice,
    key_bits: usize,
    load_value: FV,
    load_extra: FE,
) -> Result<HashmapAug<V, E>>
where
    V: Clone,
    E: Clone,
    FV: Fn(&mut Slice) -> Result<V>,
    FE: Fn(&mut Slice) -> Result<E>,
{
    let mut leaves = BTreeMap::new();
    let mut forks = Vec::new();
    let root = parse_aug_node(
        slice,
        key_bits,
        &mut Vec::new(),
        key_bits,
        &mut leaves,
        &mut forks,
        &load_value,
        &load_extra,
    )?;
    Ok(HashmapAug {
        key_bits,
        root,
        leaves,
        forks,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_aug_node<V, E, FV, FE>(
    slice: &mut Slice,
    remaining: usize,
    prefix: &mut Vec<bool>,
    key_bits: usize,
    leaves: &mut BTreeMap<BitKey, (V, E)>,
    forks: &mut Vec<HashmapAugFork<E>>,
    load_value: &FV,
    load_extra: &FE,
) -> Result<HashmapAugNode<V, E>>
where
    V: Clone,
    E: Clone,
    FV: Fn(&mut Slice) -> Result<V>,
    FE: Fn(&mut Slice) -> Result<E>,
{
    let label = load_label(slice, remaining)?;
    let label_len = label.len();
    prefix.extend(label.iter().copied());
    let node_remaining = remaining - label_len;

    let node = if node_remaining == 0 {
        let key = bit_vec_to_key(prefix, key_bits)?;
        let extra = load_extra(slice)?;
        let value = load_value(slice)?;
        if leaves
            .insert(key.clone(), (value.clone(), extra.clone()))
            .is_some()
        {
            bail!("Duplicate dictionary key");
        }
        HashmapAugNode::Leaf {
            label,
            extra,
            value,
        }
    } else {
        let fork_prefix = bit_vec_to_key(prefix, prefix.len())?;
        let left = slice.load_reference()?;
        let right = slice.load_reference()?;

        prefix.push(false);
        let mut left_slice = Slice::new(left);
        let left_node = parse_aug_node(
            &mut left_slice,
            node_remaining - 1,
            prefix,
            key_bits,
            leaves,
            forks,
            load_value,
            load_extra,
        )?;
        ensure_aug_ref_consumed(&left_slice)?;
        prefix.pop();

        prefix.push(true);
        let mut right_slice = Slice::new(right);
        let right_node = parse_aug_node(
            &mut right_slice,
            node_remaining - 1,
            prefix,
            key_bits,
            leaves,
            forks,
            load_value,
            load_extra,
        )?;
        ensure_aug_ref_consumed(&right_slice)?;
        prefix.pop();

        let extra = load_extra(slice)?;
        forks.push(HashmapAugFork {
            prefix: fork_prefix,
            extra: extra.clone(),
        });
        HashmapAugNode::Fork {
            label,
            left: Box::new(left_node),
            right: Box::new(right_node),
            extra,
        }
    };

    prefix.truncate(prefix.len() - label_len);
    Ok(node)
}

fn ensure_aug_ref_consumed(slice: &Slice) -> Result<()> {
    if slice.is_empty() {
        Ok(())
    } else {
        bail!(
            "Trailing data in HashmapAug reference: {} bits and {} refs remaining",
            slice.remaining_bits(),
            slice.remaining_refs()
        );
    }
}

fn store_aug_node<V, E, FV, FE>(
    builder: &mut Builder,
    node: &HashmapAugNode<V, E>,
    store_value: &FV,
    store_extra: &FE,
) -> Result<()>
where
    FV: Fn(&mut Builder, &V) -> Result<()>,
    FE: Fn(&mut Builder, &E) -> Result<()>,
{
    match node {
        HashmapAugNode::Leaf {
            label,
            extra,
            value,
        } => {
            store_label(builder, label, label.len())?;
            store_extra(builder, extra)?;
            store_value(builder, value)?;
        }
        HashmapAugNode::Fork {
            label,
            left,
            right,
            extra,
        } => {
            let node_remaining = node_remaining_after_label(node)?;
            store_label(builder, label, node_remaining + label.len())?;

            let mut left_builder = Builder::new();
            store_aug_node(&mut left_builder, left, store_value, store_extra)?;
            builder.store_ref(left_builder.build()?)?;

            let mut right_builder = Builder::new();
            store_aug_node(&mut right_builder, right, store_value, store_extra)?;
            builder.store_ref(right_builder.build()?)?;
            store_extra(builder, extra)?;
        }
    }
    Ok(())
}

fn node_remaining_after_label<V, E>(node: &HashmapAugNode<V, E>) -> Result<usize> {
    match node {
        HashmapAugNode::Leaf { .. } => Ok(0),
        HashmapAugNode::Fork { left, right, .. } => {
            Ok(1 + total_aug_edge_bits(left)?.max(total_aug_edge_bits(right)?))
        }
    }
}

fn total_aug_edge_bits<V, E>(node: &HashmapAugNode<V, E>) -> Result<usize> {
    match node {
        HashmapAugNode::Leaf { label, .. } => Ok(label.len()),
        HashmapAugNode::Fork { label, left, .. } => {
            Ok(label.len() + 1 + total_aug_edge_bits(left)?)
        }
    }
}

fn common_aug_prefix_len<V, E>(
    entries: &[(&BitKey, &V, &E)],
    depth: usize,
    max_len: usize,
) -> Result<usize> {
    let first = entries[0].0;
    let mut len = 0usize;
    'outer: while len < max_len {
        let bit = first.bit(depth + len)?;
        for (key, _, _) in &entries[1..] {
            if key.bit(depth + len)? != bit {
                break 'outer;
            }
        }
        len += 1;
    }
    Ok(len)
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
                builder.store_uint::<u16>(*value as u16)?;
                Ok(())
            })
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(8, |slice| slice.load_uint::<u16>())
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
                builder.store_uint_custom::<u8>(*value as u8, 1)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        assert!(
            slice
                .load_hashmap_e_with(256, |slice| slice.load_uint_custom::<u8>(1))
                .unwrap()
                .is_empty()
        );

        let mut dict = HashmapE::new(267);
        let key = BitKey::from_bits(vec![0xAA; 34], 267).unwrap();
        dict.insert_bit_key(key.clone(), 7u64).unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_e_with(&dict, |builder, value| {
                builder.store_uint_custom::<u8>(*value as u8, 4)?;
                Ok(())
            })
            .unwrap();
        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_e_with(267, |slice| slice.load_uint_custom::<u8>(4))
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
            builder.store_uint_custom::<u8>(*value as u8, 2)?;
            Ok(())
        })
        .unwrap()
        .unwrap();
        let mut broken = Builder::new();
        broken.store_bits(root.data(), root.bit_len()).unwrap();
        let broken = broken.build().unwrap();
        assert!(
            deserialize_hashmap_root(&broken, 2, |slice| slice.load_uint_custom::<u8>(2)).is_err()
        );
    }

    #[test]
    fn hashmap_aug_e_roundtrips_empty_with_top_extra() {
        let dict: HashmapAugE<u64, u64> = HashmapAugE::empty(8, 99);
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_e_with(
                8,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();

        assert!(decoded.is_empty());
        assert_eq!(*decoded.extra(), 99);
    }

    #[test]
    fn hashmap_aug_e_rejects_trailing_root_ref_data() {
        let dict = HashmapAug::from_entries(
            8,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(0xAB, 8).unwrap(),
                value: 7u64,
                extra: 11u64,
            }],
            0,
        )
        .unwrap();

        let mut root_builder = Builder::new();
        root_builder
            .store_hashmap_aug_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();
        root_builder.store_bit(true).unwrap();

        let mut builder = Builder::new();
        builder.store_bit(true).unwrap();
        builder.store_ref(root_builder.build().unwrap()).unwrap();
        builder.store_uint::<u8>(99 as u8).unwrap();

        let mut slice = builder.to_slice().unwrap();
        assert!(
            slice
                .load_hashmap_aug_e_with(
                    8,
                    |slice| slice.load_uint::<u8>(),
                    |slice| { slice.load_uint::<u8>() }
                )
                .is_err()
        );
    }

    #[test]
    fn hashmap_aug_roundtrips_single_leaf() {
        let dict = HashmapAug::from_entries(
            8,
            vec![HashmapAugLeaf {
                key: BitKey::from_u64(0xAB, 8).unwrap(),
                value: 7u64,
                extra: 11u64,
            }],
            0,
        )
        .unwrap();

        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_with(
                &dict,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_with(
                8,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(
            decoded
                .get_bit_key(&BitKey::from_u64(0xAB, 8).unwrap())
                .unwrap(),
            Some((&7, &11))
        );
    }

    #[test]
    fn hashmap_aug_roundtrips_fork_and_preserves_extras() {
        let dict = HashmapAug::from_entries(
            4,
            vec![
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b0000, 4).unwrap(),
                    value: 1u64,
                    extra: 10u64,
                },
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b0100, 4).unwrap(),
                    value: 2u64,
                    extra: 20u64,
                },
                HashmapAugLeaf {
                    key: BitKey::from_u64(0b1100, 4).unwrap(),
                    value: 3u64,
                    extra: 30u64,
                },
            ],
            77,
        )
        .unwrap();

        let wrapped = HashmapAugE::with_root(4, dict, 88).unwrap();
        let mut builder = Builder::new();
        builder
            .store_hashmap_aug_e_with(
                &wrapped,
                |builder, value| {
                    builder.store_uint::<u8>(*value as u8)?;
                    Ok(())
                },
                |builder, extra| {
                    builder.store_uint::<u8>(*extra as u8)?;
                    Ok(())
                },
            )
            .unwrap();

        let mut slice = builder.to_slice().unwrap();
        let decoded = slice
            .load_hashmap_aug_e_with(
                4,
                |slice| slice.load_uint::<u8>(),
                |slice| slice.load_uint::<u8>(),
            )
            .unwrap();
        let root = decoded.root().unwrap();
        let leaves: Vec<_> = root
            .iter()
            .map(|(key, value, extra)| (key.to_u64().unwrap(), *value, *extra))
            .collect();
        assert_eq!(leaves, vec![(0, 1, 10), (4, 2, 20), (12, 3, 30)]);
        assert_eq!(*decoded.extra(), 88);
        assert!(root.fork_extras().iter().all(|fork| fork.extra == 77));
    }

    #[test]
    fn hashmap_aug_rejects_empty_duplicate_and_wrong_width() {
        assert!(HashmapAug::<u64, u64>::from_entries(4, vec![], 0).is_err());
        assert!(
            HashmapAug::from_entries(
                4,
                vec![HashmapAugLeaf {
                    key: BitKey::from_u64(0, 5).unwrap(),
                    value: 1u64,
                    extra: 1u64,
                }],
                0,
            )
            .is_err()
        );
        assert!(
            HashmapAug::from_entries(
                4,
                vec![
                    HashmapAugLeaf {
                        key: BitKey::from_u64(1, 4).unwrap(),
                        value: 1u64,
                        extra: 1u64,
                    },
                    HashmapAugLeaf {
                        key: BitKey::from_u64(1, 4).unwrap(),
                        value: 2u64,
                        extra: 2u64,
                    },
                ],
                0,
            )
            .is_err()
        );
    }
}
