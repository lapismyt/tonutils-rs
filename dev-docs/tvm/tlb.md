# TLB Data Models

TLB defines the bit-level schema language for TVM cells. It is separate from TL, which defines byte-level protocol messages.

## Difference From TL

| TL | TLB |
| --- | --- |
| byte-level protocol serialization | bit-level cell serialization |
| used for ADNL, LiteAPI, DHT | used for blocks, messages, accounts, state |
| constructor ids are 32-bit TL ids | constructors are bit tags or implicit |
| vectors and bytes are TL primitives | refs, bits, Maybe, Either, HashmapE are common |

## Common TLB Forms

- `Maybe X`: one bit presence marker plus value if present.
- `Either X Y`: one bit branch marker plus selected value.
- `^X`: reference to a cell containing X.
- `HashmapE n X`: optional Patricia-tree dictionary. `0` means empty; `1` is followed by a reference to a `Hashmap n X` root edge. Keys are fixed-width MSB-first bitstrings, edges store `HmLabel` compressed prefixes, and fork children are references selected by the next key bit.
- `VarUInteger n`: variable-width integer with length prefix.

## HashmapE Mapping

`src/tvm/dict.rs` implements the first generic dictionary foundation:

- `BitKey` stores fixed-width key bits with canonical zeroed unused final-byte bits.
- `HashmapE<V>` stores sorted `BitKey` entries and serializes them as canonical TL-B `HashmapE`.
- `Builder::store_hashmap_e_with` and `Slice::load_hashmap_e_with` encode and decode values through callbacks, leaving the concrete `X` model to callers.

Serialization emits canonical `HmLabel` forms (`hml_short`, `hml_long`, or `hml_same`). Deserialization accepts all valid label forms and rejects overlong labels, missing fork references, duplicate decoded keys, and key-width mismatches.

## Needed TON Models

Core models to implement:

- `CommonMsgInfo`,
- internal and external messages,
- `StateInit`,
- `Account`,
- `ShardAccount`,
- `Transaction`,
- transaction phases,
- block header,
- value flow,
- shard hashes,
- config params.

## Testing Strategy

- Decode known BoC fixtures.
- Re-encode and compare cell hashes.
- Validate field-level values against tonutils-go or official tools.
- Add negative tests for invalid tags and underflow.
