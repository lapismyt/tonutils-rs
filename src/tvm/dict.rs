//! TON HashmapE and HashmapAugE dictionary support.
//!
//! TON dictionaries are canonical Patricia trees over fixed-width bitstring
//! keys. `HashmapE n X` stores either `hme_empty$0` or `hme_root$1` followed by a
//! reference to a `Hashmap n X` edge.

include!("dict_parts/part1.rs");
include!("dict_parts/part2.rs");
include!("dict_parts/part3.rs");
