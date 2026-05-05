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

## Exotic Cells

Types still required:

- pruned branch,
- library reference,
- Merkle proof,
- Merkle update.

Exotic level and hash rules differ from ordinary cells. Proof verification depends on them.

## Crate Mapping

- `src/tvm/cell.rs`: `Cell`, `CellBuilder`.
- `src/tvm/builder.rs`: convenience builder.
- `src/tvm/slice.rs`: reader.

## Tests Needed

- descriptor fixtures,
- top-up bit fixtures,
- hash fixtures from known cells,
- exotic cell fixtures,
- overflow and underflow tests.
