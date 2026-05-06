# TVM Cells

Cells are the primary storage and serialization unit in TON.

## Ordinary Cell Limits

- Data bits: `0..=1023`.
- References: `0..=4`.
- Level: `0..=3`.
- Exotic flag: false.

## Descriptor Bytes

First byte:

```text
refs_count + 8 * exotic + 32 * level
```

Second byte:

```text
floor(bits / 8) + ceil(bits / 8)
```

## Top-Up Bit

If data bit length is not divisible by 8, serialized data includes a `1` top-up bit immediately after the last data bit, then zero bits until byte boundary.

## Fixed-Width Integer Bits

TLB `uintN` values are stored as exactly `N` bits in big-endian bit order, most significant bit first. When `N` is not divisible by 8, the first value bit still occupies the next available high bit in the target byte; unused bits in the final byte are zero in the in-memory cell data and are separate from the serialized top-up bit.

Unsigned fixed-width values must satisfy `0 <= value < 2^N`. `N = 0` is valid only for zero.

TLB `intN` values use fixed-width two's-complement encoding. A signed value must be in:

```text
-2^(N - 1) <= value <= 2^(N - 1) - 1
```

For `N = 0`, only zero is valid. Readers decode `intN` by reading the unsigned bit string, checking bit `N - 1`, and subtracting `2^N` when the sign bit is set.

`CellBuilder::store_big_uint`, `CellBuilder::store_big_int`, `Builder::store_big_uint`, `Builder::store_big_int`, `Slice::load_big_uint`, and `Slice::load_big_int` support widths up to the ordinary cell data limit of `1023` bits. The existing `u64` and `i64` helpers keep their signatures and delegate to the same canonical encoding rules.

## VarUInteger

TLB `VarUInteger n` stores an unsigned byte length in `n` bits, followed by exactly that many big-endian value bytes. Zero is encoded as a zero length and no value bytes. The byte length itself must fit in the `n`-bit length field.

TON coin amounts use `VarUInteger 16`, which means a 4-bit byte length followed by at most 15 value bytes. Values requiring 16 bytes are invalid even if they fit in Rust `u128`.

## Depth

- No refs: depth `0`.
- With refs: `1 + max(ref.depth)`.
- Serialized as two-byte big-endian values in representation hash calculation.

## Representation Hash

For ordinary cells:

1. descriptors,
2. top-up-padded data,
3. reference depths,
4. reference hashes,
5. SHA-256.

Golden fixtures for ordinary cells are checked directly against this preimage,
not against BoC bytes:

| Cell | Representation preimage | SHA-256 representation hash |
| --- | --- | --- |
| Empty ordinary cell | `0000` | `96a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7` |
| One-bit ordinary cell containing `1` | `0001c0` | `7c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0` |
| Full-byte ordinary cell containing `ab` | `0002ab` | `57c2a1a13baa2762109ed68be0c396f2303ce17e3dde7917d0e74b4072b1dbc7` |
| 32-bit ordinary cell containing `0000000f` | `00080000000f` | `57b520dbcb9d135863fc33963cde9f6db2ded1430d88056810a2c9434a3860f9` |
| One-bit root containing `1`, with refs to the empty and one-bit fixtures | `0201c00000000096a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc77c6c1a965fd501d2938c2c0e06626bdaa3531357016e169070c9ef79c4c46bc0` | `383598f93bde0afbe68b632ae75d5ffa6747df1284e2f4abb86cd2c5840514fe` |
| One-bit middle containing `1`, with ref to the empty fixture | `0101c0000096a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc7` | `9770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b` |
| Full-byte root containing `ab`, with ref to the one-bit middle fixture | `0102ab00019770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b` | `9f19f1fa052329a70f79c2adaef4e9f4e73eb88be389918473adc5f9a2801181` |
| Full-byte root containing `ab`, with refs to the empty and one-bit middle fixtures | `0202ab0000000196a296d224f285c67bee93c30f8a309157f0daa35dc5b87e410b78630a09cfc79770d42f6d781e048a432b849b56d5329de4667b37cfb918429a23f90cb9884b` | `6d112e22e9b4f47922b27cb78ffb8c4c3be4be304cdcb9ad24560e3104827eb6` |

The multi-level fixtures above keep ordinary-cell depth handling explicit:

- the empty leaf has depth `0`, descriptor `0000`, and no child data,
- the one-bit middle cell has depth `1`, descriptor `0101`, and child depth bytes `0000`,
- the full-byte chained root has depth `2`, descriptor `0102`, and child depth bytes `0001`,
- the two-reference root has depth `2`, descriptor `0202`, and child depth bytes `0000 0001` before both child hashes.

## Exotic Cells

The crate models supported exotic cells explicitly through `ExoticCellKind`.
Ordinary constructors such as `Cell::new()` and `Cell::with_data()` always
create ordinary cells. Exotic cells are constructed by BoC decoding or by
`Cell::with_exotic_data(data, bit_len, references)`, which validates the tag,
payload length, reference count, and derived level.

The first byte of every exotic cell data payload is the type tag:

| Tag | Kind | Payload | References | Derived level |
| --- | --- | --- | --- | --- |
| `0x01` | Pruned branch | tag, one-byte level mask `1..=7`, one 32-byte hash for each set mask bit, then one two-byte big-endian depth for each set mask bit | `0` | index of the most significant set mask bit plus one, therefore `1..=3` |
| `0x02` | Library reference | tag plus a 32-byte library cell representation hash, exactly `264` bits | `0` | `0` |
| `0x03` | Merkle proof | tag, one 32-byte proof hash, one two-byte big-endian proof depth, exactly `280` bits | `1` | `max(ref.level - 1, 0)` |
| `0x04` | Merkle update | tag, old 32-byte proof hash, new 32-byte proof hash, old two-byte depth, new two-byte depth, exactly `552` bits | `2` | `max(old_ref.level - 1, new_ref.level - 1, 0)` |

The BoC descriptor still carries the exotic flag outside the data bits:

```text
refs_count + 8 * 1 + 32 * derived_level
```

BoC decoding rejects an exotic cell if the descriptor level does not match the
level derived from its kind-specific rules. The decoder also rejects unsupported
exotic tags, tags missing from short payloads, invalid pruned branch masks,
wrong reference counts, and wrong exact payload lengths.

`Cell::hash()` remains the SHA-256 representation hash of the serialized cell
representation, including the exotic descriptor bit and top-up-padded data.
`Cell::depth()` remains the representation depth of the decoded cell graph:
pruned and library cells have no references and therefore depth `0`; Merkle
proof and Merkle update cells have graph depth based on their explicit
references. Kind-specific proof depths and pruned-branch depths are available
through `ExoticCellKind`; callers must use those fields for proof semantics
instead of treating `Cell::depth()` as the deleted subtree depth.

The implementation does not yet expose `hash_i`/`depth_i` multi-level helpers.
Proof verification code must add those helpers before validating higher hashes
with `CHASHI`/`CHASHIX`-style semantics.

## Crate Mapping

- `src/tvm/cell.rs`: `Cell`, `CellBuilder`.
- `src/tvm/builder.rs`: convenience builder.
- `src/tvm/slice.rs`: reader.

## Tests Needed

- overflow and underflow tests.
- upstream or pytoniq-core golden fixtures for exotic cells and BoC bytes.
