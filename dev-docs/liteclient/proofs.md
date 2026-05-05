# Light Client Proof Verification

LiteAPI can return proof bytes, but the SDK must verify them before claiming trustless correctness.

## Trust Anchors

A light client needs:

- trusted zerostate,
- trusted or verified validator set,
- verified masterchain block chain,
- verified shard links.

## Proof Types

Common proof material:

- block proof,
- shard block proof,
- account state proof,
- config proof,
- validator signatures.

## Verification Outline

1. Decode proof BoC.
2. Verify cell hashes against expected block ids.
3. Verify Merkle proof exotic cells.
4. Verify validator set and signatures.
5. Verify shard inclusion in masterchain.
6. Verify account state in shard state.

## Current State

The crate exposes proof bytes but does not yet fully verify them.

## Missing Work

- Exotic cell support.
- Validator set TLB decoding.
- Signature set verification.
- Block proof path validation.
- Account proof validation.
