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

LiteAPI proof byte fields are BoCs. Some account-state proof payloads have
multiple roots; for `liteServer.getAccountState`, the `state` byte field is the
actual TL-B `Account` cell, while `proof` contains pruned proof material that
must be combined with that state cell during verification.

## Verification Outline

1. Decode proof BoC.
2. Verify cell hashes against expected block ids.
3. Verify Merkle proof exotic cells.
4. Verify validator set and signatures.
5. Verify shard inclusion in masterchain.
6. Verify account state in shard state.

## Current State

The crate exposes proof bytes but does not yet fully verify them.
BoC diagnostics inspect single-root and multi-root proof payloads structurally
and report root counts and representation hashes. This inspection does not
construct TL-B proof objects and is not proof verification. Live LiteServer
`getAccountState` responses have been smoke-tested for multi-root proof BoC
structural inspection in the CLI account command.

No current LiteClient or CLI API should be described as trustless or verified
unless its name and documentation explicitly say "verified". The current APIs
inspect, decode, and preserve proof material so future proof-specific work can
anchor it correctly; full proof verification is intentionally deferred.

`liteclient::boc::extract_verified_shard_account` is the current checked
extraction boundary for high-level transaction unblocking. It accepts proof
material whose root has already been anchored to a verified shard-account path,
decodes `ShardAccount`, verifies the requested standard account hash, rejects
state/proof mismatches, and can check an independently verified shard root hash.
It is not yet a full `ShardAccounts` dictionary traversal or Merkle proof
verifier.

## Missing Work

- Validator set TLB decoding.
- Signature set verification.
- Block proof path validation.
- Account proof validation.
- Traverse `ShardAccounts` dictionaries from live account proofs after shard
  and Merkle proof roots are verified.
