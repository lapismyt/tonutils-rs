# TON Dictionaries

## Purpose And Scope

TON dictionaries are TL-B hashmaps over fixed-width bitstring keys. They are not equivalent to Rust `HashMap` serialization: the wire format is a canonical Patricia tree whose edges compress shared key prefixes.

This page documents the `HashmapE n X`, `HashmapAug n X Y`, and
`HashmapAugE n X Y` foundation implemented in `src/tvm/dict.rs`. It covers
fixed-width keys, canonical label serialization, fork nodes, augmentation
preservation, and callback-based value codecs. It does not cover Merkle proofs,
exotic cells, or full blockchain model decoding.

## Wire Format And Data Model

`HashmapE n X` is defined by the upstream TON TL-B schema as:

- `hme_empty$0 = HashmapE n X`
- `hme_root$1 root:^(Hashmap n X) = HashmapE n X`

The referenced `Hashmap n X` is an edge:

- `hm_edge#_ label:(HmLabel ~l n) {n = (~m) + l} node:(HashmapNode m X) = Hashmap n X`

The node is either a leaf or a fork:

- `hmn_leaf#_ value:X = HashmapNode 0 X`
- `hmn_fork#_ left:^(Hashmap n X) right:^(Hashmap n X) = HashmapNode (n + 1) X`

An edge label consumes a common prefix before the node. A fork consumes one additional key bit by selecting the left child for `0` and the right child for `1`.

Augmented dictionaries use the same edge and label structure, but store
augmentation values at every node:

- `ahm_edge#_ label:(HmLabel ~l n) node:(HashmapAugNode m X Y) = HashmapAug n X Y`
- `ahmn_leaf#_ extra:Y value:X = HashmapAugNode 0 X Y`
- `ahmn_fork#_ left:^(HashmapAug n X Y) right:^(HashmapAug n X Y) extra:Y = HashmapAugNode (n + 1) X Y`
- `ahme_empty$0 extra:Y = HashmapAugE n X Y`
- `ahme_root$1 root:^(HashmapAug n X Y) extra:Y = HashmapAugE n X Y`

`HashmapAug n X Y` is non-empty. `HashmapAugE n X Y` may be empty, but the
empty constructor still stores the top-level `extra:Y`.

## Labels

`HmLabel` has three valid encodings:

- `hml_short$0 len:(Unary ~n) s:(n * Bit)`
- `hml_long$10 n:(#<= m) s:(n * Bit)`
- `hml_same$11 v:Bit n:(#<= m)`

The `#<= m` length field uses `ceil(log2(m + 1))` bits. For example, a label whose maximum remaining length is `267` stores the length in `9` bits.

Serializers must emit a canonical label. This crate evaluates all valid encodings, chooses the shortest encoded bitstring, and on equal encoded length chooses the lexicographically smallest encoded bitstring. Deserializers accept all valid label forms and reject labels whose decoded length exceeds the current remaining key width.

## Invariants And Edge Cases

- All keys in one dictionary have exactly `n` bits.
- Key bits are stored MSB-first, and unused bits in the final byte are zero.
- Empty dictionaries serialize as a single `0` bit and no reference.
- Non-empty dictionaries serialize as `1` plus one root reference.
- `HashmapAug` has no empty constructor; use `HashmapAugE` when an empty
  dictionary is valid.
- `HashmapAugE` stores a top-level augmentation after the empty/root bit and
  optional root reference.
- A leaf is valid only after exactly `n` key bits have been reconstructed.
- A fork must have both child references.
- Duplicate decoded keys are rejected.
- Slice and reference underflow errors are propagated from `Slice`.
- Values are encoded by caller-provided callbacks; dictionary code does not infer `X`.
- Augmentation values are schema-specific. The dictionary layer preserves
  decoded leaf, fork, and top-level extras, but does not compute aggregate
  values for SDK-created dictionaries.

## Current Crate Mapping

`BitKey` stores canonical fixed-width keys. `HashmapE<V>` stores entries in
`BTreeMap<BitKey, V>` so serialization is deterministic. `HashmapAug<V, E>`
stores a non-empty augmented tree plus key-ordered leaves, and
`HashmapAugE<V, E>` wraps an optional augmented root with the top-level extra.

`Builder::store_hashmap_e_with` and `Slice::load_hashmap_e_with` provide the generic codec surface:

- the store callback receives `&mut Builder` and `&V`,
- the load callback receives `&mut Slice` and returns `V`.

The augmented APIs mirror this shape and add an augmentation callback:

- `Builder::store_hashmap_aug_with`
- `Builder::store_hashmap_aug_e_with`
- `Slice::load_hashmap_aug_with`
- `Slice::load_hashmap_aug_e_with`

`Dict`, `DictKey`, and `DictValue` remain compatibility wrappers. Integer keys use fixed-width `BitKey` conversion. Address keys serialize the full 267-bit `addr_std` form rather than truncating through `u64`.

## Missing Work

- Proof-friendly dictionary traversal and path extraction.
- Golden fixtures compared against official TON implementations.
- Higher-level TL-B model codecs for full blocks, shard state, and config values.
- Cell capacity strategies for large inline values beyond the caller's callback behavior.
