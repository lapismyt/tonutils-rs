# Public API Design

The SDK should expose both low-level protocol access and ergonomic high-level operations.

## API Layers

Low-level:

- TL request and response structs,
- ADNL peer,
- TVM cells and BoC,
- raw LiteAPI query bytes.

Mid-level:

- `LiteClient`,
- `LiteBalancer`,
- typed LiteAPI methods,
- TVM stack values.

High-level:

- `Contract`,
- wallet helpers,
- jetton helpers,
- NFT helpers,
- mempool scanner.

## Naming Rules

- Use TON protocol names when exposing wire-level types.
- Use Rust idioms for high-level builders and helpers.
- Avoid long boolean parameter lists for flag-heavy methods; prefer options structs.

## Ownership Rules

- Use borrowed inputs for bytes and cells where it reduces allocations without complicating API.
- Return owned values from network boundaries.
- Preserve raw bytes alongside decoded values when decoding can be lossy or incomplete.

## Compatibility

The crate is pre-stable, so breaking changes are acceptable. However, every breaking change should make the API closer to:

- explicit trust assumptions,
- fewer hidden allocations,
- better feature gating,
- schema compatibility.

## Missing Work

- Options structs for LiteAPI flag-heavy methods.
- Shared trait for LiteClient and LiteBalancer contract execution.
- Raw response preserving wrappers.
