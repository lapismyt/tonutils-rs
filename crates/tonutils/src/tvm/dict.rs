//! TON HashmapE and HashmapAugE dictionary support.
//!
//! TON dictionaries are canonical Patricia trees over fixed-width bitstring
//! keys. `HashmapE n X` stores either `hme_empty$0` or `hme_root$1` followed by a
//! reference to a `Hashmap n X` edge.

pub(super) use crate::tvm::TvmResult as Result;
#[cfg(test)]
pub(super) use crate::tvm::address::Address;
pub(super) use crate::tvm::builder::Builder;
pub(super) use crate::tvm::error::dictionary_bail as bail;
pub(super) use crate::tvm::slice::Slice;
mod augmented;
mod bit_key;
mod bits;
mod compat;
mod hashmap;
mod labels;
#[cfg(test)]
mod tests;

use hashmap::HashmapAugNode;
pub use hashmap::{
    BitKey, Dict, DictKey, DictValue, HashmapAug, HashmapAugE, HashmapAugFork, HashmapAugLeaf,
    HashmapE,
};

#[cfg(test)]
use hashmap::{deserialize_hashmap_root, serialize_hashmap_root};
use labels::{
    bits_to_bytes, build_aug_node, clear_unused_bits, collect_key_bits, common_prefix_len,
    deserialize_hashmap_aug_from_slice, ensure_aug_ref_consumed, get_bit, parse_edge, set_bit,
    store_aug_node, store_label,
};
#[cfg(test)]
use labels::{canonical_label, load_label};
