# Bag Of Cells

BoC serializes a directed acyclic graph of cells into bytes.

## Responsibilities

BoC must preserve:

- root cells,
- cell data bits,
- references,
- topological order,
- optional index table,
- optional CRC32C.

## Header Concepts

A BoC header contains:

- magic prefix,
- flags,
- size byte widths,
- cell count,
- root count,
- absent count,
- total cell size,
- root indexes,
- optional index offsets.

## Cell Serialization

Each cell serializes:

1. descriptor bytes,
2. padded data bytes,
3. reference indexes.

Reference indexes point to other serialized cells.

## Validation Rules

Decoder must reject:

- unknown magic,
- truncated header,
- unsupported flags,
- root index out of range,
- reference index out of range,
- duplicate or cyclic invalid structure,
- CRC mismatch,
- trailing bytes when not allowed.

## Crate Mapping

- `src/tvm/boc.rs`

## Missing Work

- Full support for all common BoC magic variants.
- Index table modes.
- Cache bits.
- More malformed fixture tests.
