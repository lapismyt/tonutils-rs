# TVM Primitives

The `tvm` feature enables TON cells, builders, slices, BoC helpers, addresses,
dictionaries, and TVM stack values. It also enables `tl` because several public
types map cells and addresses into LiteAPI structures.

Audience: users working with offline cells, BoC payloads, TL-B models, addresses,
or get-method stack values. Prerequisites: `tvm` feature only; no live network
access is required unless the BoC or stack payload came from LiteClient calls.

## Cells, Builders, And Slices

A cell stores up to 1023 bits and up to 4 references. `Builder` and
`CellBuilder` write values into a cell, and `Slice` reads them back in order.

```rust
use num_bigint::BigUint;
use tonutils::tvm::{Builder, Slice};

fn example() -> anyhow::Result<()> {
    let value = BigUint::from(1u64) << 96;
    let mut builder = Builder::new();
    builder.store_u32(0x12345678)?;
    builder.store_big_uint(&value, 128)?;
    let cell = builder.end_cell()?;

    let mut slice = Slice::new(cell);
    assert_eq!(slice.load_u32()?, 0x12345678);
    assert_eq!(slice.load_big_uint(128)?, value);
    Ok(())
}
```

Reads and writes are bounds checked. Loading too many bits or references returns
an error instead of silently truncating data.

## BoC

BoC helpers serialize and parse a root cell:

```rust
use tonutils::tvm::{Builder, deserialize_boc, serialize_boc};

fn example() -> anyhow::Result<()> {
    let mut builder = Builder::new();
    builder.store_byte(7)?;
    let cell = builder.end_cell()?;
    let boc = serialize_boc(&cell, true)?;
    let decoded = deserialize_boc(&boc)?;
    assert_eq!(decoded.hash(), cell.hash());
    Ok(())
}
```

Convenience helpers convert BoC data to and from hex and base64. Current BoC
support covers the crate's ordinary-cell use cases; index table modes, cache
bits, exotic cells, and official golden fixture coverage are still being
expanded.

## Addresses

`Address` parses raw `workchain:hash` strings and user-friendly base64 forms,
and can convert to LiteAPI `AccountId`:

```rust
use std::str::FromStr;
use tonutils::tvm::Address;

fn example(address: &str) -> anyhow::Result<()> {
    let parsed = Address::from_str(address)?;
    let account = parsed.to_account_id();
    println!("{} {}", account.workchain, parsed.to_hex());
    Ok(())
}
```

Address formatting exposes bounceable and test-only flags. Callers should keep
test-only addresses out of mainnet workflows.

## Dictionaries

`HashmapE` stores fixed-width `BitKey` values with callback-based value codecs.
The higher-level `Dict` wrapper supports integer keys and cell values for the
current public surface.

```rust
use tonutils::tvm::{Dict, DictValue};

fn example() -> anyhow::Result<()> {
    let mut dict = Dict::new(16);
    dict.set_int_key(7, DictValue::Uint(42, 32))?;
    let root = dict.serialize()?;
    assert!(root.is_some());
    Ok(())
}
```

The dictionary encoder implements canonical labels and fork nodes. Official
TON golden fixtures, proof-friendly traversal, and typed TL-B value codecs
remain TODO items.

## Stack Values

`TvmStack` is used by contract get-method helpers. It supports nulls, integers,
cells, slices, tuples, lists, and explicit unsupported payloads for lossless
roundtrips.

```rust
use tonutils::tvm::{TvmStack, TvmStackEntry};

fn example() -> anyhow::Result<()> {
    let stack = TvmStack::new(vec![TvmStackEntry::int(10)]);
    let boc = stack.to_boc()?;
    let decoded = TvmStack::from_boc(&boc)?;
    assert_eq!(decoded.entries().len(), 1);
    Ok(())
}
```

Compatibility with every liteserver `runSmcMethod` return shape is still being
verified with live and golden fixtures.
