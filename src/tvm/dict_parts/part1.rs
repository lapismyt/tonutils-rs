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

    /// Removes a value by fixed-width bit key.
    pub fn remove_bit_key(&mut self, key: &BitKey) -> Result<Option<V>> {
        if key.bit_len() != self.key_bits {
            bail!(
                "Dictionary key length {} does not match {}",
                key.bit_len(),
                self.key_bits
            );
        }
        Ok(self.map.remove(key))
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

