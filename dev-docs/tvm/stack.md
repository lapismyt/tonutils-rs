# TVM Stack

TVM stack values are used for smart-contract get-method parameters and results.

## Required Entry Types

- null,
- integer,
- cell,
- slice,
- tuple,
- list,
- unsupported preserved value.

## Integer Size

TVM integers are not limited to 64 bits. The current crate has an initial `i64` representation and must move to arbitrary precision for compatibility.

## Cells And Slices

Cells and slices should preserve:

- data bits,
- refs,
- current slice offset where applicable,
- BoC compatibility.

## Tuples And Lists

Tuples and lists can nest. Implementation must avoid the four-reference direct-cell limit by using linked or referenced representation compatible with liteserver expectations.

## LiteAPI Relation

`liteServer.runSmcMethod` sends `params:bytes` and receives `result:mode.2?bytes`. These bytes must match TON stack serialization, not an arbitrary SDK-local format.
The root `VmStack` cell starts with `depth:(## 24)`, followed by the stack list
payload when depth is non-zero. Empty get-method calls therefore serialize to a
BoC whose root cell contains exactly 24 zero bits and no references.

## Current Crate Mapping

- `src/tvm/stack.rs`
- `src/liteclient/client.rs` get-method helpers

## Missing Work

- Verify non-empty stack encoding against live liteserver.
- Decode real result stacks.
- Support arbitrary precision integers.
- Support deep tuples and lists.
