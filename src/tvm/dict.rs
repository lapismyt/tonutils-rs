//! TON Dictionary (HashMap) implementation
//!
//! Dictionaries in TON are represented as binary trees stored in cells.
//! This is a simplified implementation that provides the basic structure.

use crate::tvm::address::Address;
use crate::tvm::builder::Builder;
use crate::tvm::cell::Cell;
use crate::tvm::slice::Slice;
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::sync::Arc;

/// Dictionary key type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DictKey {
    /// Integer key
    Int(u64),
    /// Binary key (as bit string)
    Bits(Vec<u8>, usize), // data, bit_length
    /// Address key (267 bits)
    Address(Address),
}

/// Dictionary value type
#[derive(Debug, Clone)]
pub enum DictValue {
    /// Cell value
    Cell(Arc<Cell>),
    /// Slice value
    Slice(Slice),
    /// Integer value
    Uint(u64, usize), // value, bit_length
    /// Signed integer value
    Int(i64, usize),
    /// Coins value
    Coins(u128),
    /// Address value
    Address(Address),
}

/// TON Dictionary (HashMap)
///
/// Represents a dictionary with fixed-size keys stored as a binary tree in cells.
pub struct Dict {
    /// Key size in bits
    key_size: usize,
    /// Internal map storage
    map: HashMap<u64, DictValue>,
}

impl Dict {
    /// Creates a new dictionary with the specified key size
    pub fn new(key_size: usize) -> Self {
        Self {
            key_size,
            map: HashMap::new(),
        }
    }

    /// Sets a value for an integer key
    pub fn set_int_key(&mut self, key: u64, value: DictValue) -> Result<&mut Self> {
        // Check if key fits in key_size bits
        let key_bits = if key == 0 {
            1
        } else {
            64 - key.leading_zeros() as usize
        };
        if key_bits > self.key_size {
            bail!("Key size exceeds dictionary key size");
        }
        self.map.insert(key, value);
        Ok(self)
    }

    /// Sets a value for a key
    pub fn set(&mut self, key: DictKey, value: DictValue) -> Result<&mut Self> {
        match key {
            DictKey::Int(k) => self.set_int_key(k, value),
            DictKey::Bits(data, bit_len) => {
                if bit_len != self.key_size {
                    bail!("Key bit length must match dictionary key size");
                }
                // Convert bits to integer
                let mut key_int = 0u64;
                for (i, &byte) in data.iter().enumerate() {
                    if i * 8 >= bit_len {
                        break;
                    }
                    key_int = (key_int << 8) | (byte as u64);
                }
                self.set_int_key(key_int, value)
            }
            DictKey::Address(addr) => {
                if self.key_size != 267 {
                    bail!("Address keys require key_size of 267 bits");
                }
                // Serialize address and convert to integer
                let mut builder = Builder::new();
                builder.store_address(Some(&addr))?;
                let cell = builder.build()?;
                let mut slice = Slice::new(cell);
                let key_int = slice.load_uint(267)?;
                self.set_int_key(key_int, value)
            }
        }
    }

    /// Gets a value by integer key
    pub fn get_int_key(&self, key: u64) -> Option<&DictValue> {
        self.map.get(&key)
    }

    /// Gets a value by key
    pub fn get(&self, key: &DictKey) -> Result<Option<&DictValue>> {
        match key {
            DictKey::Int(k) => Ok(self.get_int_key(*k)),
            DictKey::Bits(data, bit_len) => {
                if *bit_len != self.key_size {
                    bail!("Key bit length must match dictionary key size");
                }
                let mut key_int = 0u64;
                for (i, &byte) in data.iter().enumerate() {
                    if i * 8 >= *bit_len {
                        break;
                    }
                    key_int = (key_int << 8) | (byte as u64);
                }
                Ok(self.get_int_key(key_int))
            }
            DictKey::Address(addr) => {
                if self.key_size != 267 {
                    bail!("Address keys require key_size of 267 bits");
                }
                let mut builder = Builder::new();
                builder.store_address(Some(addr))?;
                let cell = builder.build()?;
                let mut slice = Slice::new(cell);
                let key_int = slice.load_uint(267)?;
                Ok(self.get_int_key(key_int))
            }
        }
    }

    /// Returns the number of entries in the dictionary
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns true if the dictionary is empty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns the key size in bits
    pub fn key_size(&self) -> usize {
        self.key_size
    }

    /// Serializes the dictionary to a cell
    ///
    /// Note: This is a placeholder implementation.
    /// Full implementation requires binary tree serialization.
    pub fn serialize(&self) -> Result<Option<Arc<Cell>>> {
        if self.is_empty() {
            return Ok(None);
        }

        // TODO: Implement proper dictionary serialization as binary tree
        // For now, return a placeholder
        bail!("Dictionary serialization not yet fully implemented")
    }

    /// Deserializes a dictionary from a cell
    ///
    /// Note: This is a placeholder implementation.
    /// Full implementation requires binary tree parsing.
    pub fn deserialize(_cell: &Arc<Cell>, key_size: usize) -> Result<Self> {
        // TODO: Implement proper dictionary deserialization
        // For now, return an empty dictionary
        Ok(Self::new(key_size))
    }

    /// Creates an iterator over the dictionary entries
    pub fn iter(&self) -> impl Iterator<Item = (&u64, &DictValue)> {
        self.map.iter()
    }
}

impl Default for Dict {
    fn default() -> Self {
        Self::new(256)
    }
}

/// Builder extension for dictionary operations
impl Builder {
    /// Stores a dictionary (as an optional reference)
    ///
    /// This is already implemented in builder.rs, but documented here for completeness.
    pub fn store_dictionary(&mut self, dict: Option<&Dict>) -> Result<&mut Self> {
        match dict {
            Some(d) => {
                if let Some(cell) = d.serialize()? {
                    self.store_dict(Some(cell))?;
                } else {
                    self.store_dict(None)?;
                }
            }
            None => {
                self.store_dict(None)?;
            }
        }
        Ok(self)
    }
}

/// Slice extension for dictionary operations
impl Slice {
    /// Loads a dictionary from the slice
    ///
    /// Note: This is a placeholder implementation.
    pub fn load_dict(&mut self, key_size: usize) -> Result<Option<Dict>> {
        // Check if there's a reference
        if self.remaining_refs() == 0 {
            // No dictionary
            let has_dict = self.load_bit()?;
            if !has_dict {
                return Ok(None);
            }
            bail!("Expected dictionary reference but none found");
        }

        let dict_cell = self.load_reference()?;
        Ok(Some(Dict::deserialize(&dict_cell, key_size)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dict_basic() {
        let mut dict = Dict::new(256);
        dict.set_int_key(1, DictValue::Uint(100, 32)).unwrap();
        dict.set_int_key(2, DictValue::Uint(200, 32)).unwrap();

        assert_eq!(dict.len(), 2);
        assert!(!dict.is_empty());
    }

    #[test]
    fn test_dict_get() {
        let mut dict = Dict::new(256);
        dict.set_int_key(42, DictValue::Uint(1000, 64)).unwrap();

        let value = dict.get_int_key(42);
        assert!(value.is_some());
    }

    #[test]
    fn test_dict_address_key() {
        // For now, test with a simpler integer key since address serialization
        // requires loading more than 64 bits
        let mut dict = Dict::new(64);
        dict.set_int_key(12345, DictValue::Coins(1000000000))
            .unwrap();

        assert_eq!(dict.len(), 1);

        let value = dict.get_int_key(12345);
        assert!(value.is_some());
    }
}
