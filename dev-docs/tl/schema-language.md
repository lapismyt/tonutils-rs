# TL Schema Language

TL, the Type Language, defines binary wire formats used by TON. Schemas define both data constructors and function constructors.

## Constructor Shape

General form:

```tl
name field:type field2:type2 = ResultType;
```

Example:

```tl
tonNode.blockIdExt workchain:int shard:long seqno:int root_hash:int256 file_hash:int256 = tonNode.BlockIdExt;
```

## Constructor Id

Boxed constructors carry a 32-bit id. It can be explicit:

```tl
liteServer.signatureSet.ordinary#f644a6e6 ... = liteServer.SignatureSet;
```

If omitted, the id is CRC32 of the normalized constructor string.

Implementation requirement:

- ids in Rust annotations must match schema ids,
- schema parser tests should recompute ids and fail on drift,
- string-based ids in macros need enough scheme context to compute correctly.

## Primitive Types

| TL type | Wire meaning |
| --- | --- |
| `int` | 32-bit integer |
| `long` | 64-bit integer |
| `int128` | 16 bytes |
| `int256` | 32 bytes |
| `bytes` | length-prefixed bytes with 4-byte alignment padding |
| `string` | same binary family as bytes, interpreted as UTF-8 by convention |
| `Bool` | boxed boolean constructors |
| `vector` | vector constructor, count, then items |

## Bytes Padding

TL bytes are padded to a 4-byte boundary. The payload length is not the same as the serialized field length. Nested `bytes` fields that contain serialized TL objects must preserve this wrapping exactly.

## Boxed Vs Bare

Boxed values include constructor ids. Bare values do not. Enums are typically boxed. Struct fields are often bare unless the field type is an abstract result type.

## Flags

Flags use:

```tl
mode:# field:mode.0?bytes other:mode.2?int = Type;
```

Rules:

- `mode:#` serializes a 32-bit flags integer.
- Bit `N` controls fields marked `mode.N?`.
- Multiple fields can share the same bit.
- Optional fields are serialized in schema order only when present.

## Functions

Functions appear after `---functions---`. They are serialized as boxed constructors when called. Their output type tells the expected response abstract type.

LiteAPI wraps functions inside `liteServer.query`, then ADNL wraps that inside `adnl.message.query`.

## Rust Mapping Checklist

- Choose exact signedness and width.
- Use `[u8; 32]` or equivalent for `int256`.
- Use `Vec<T>` for vectors when allocation is acceptable.
- Use `Option<T>` for flag-controlled fields.
- Use boxed enums for abstract result types.
- Add roundtrip and constructor id tests for every new type.
