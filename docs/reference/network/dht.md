# TON DHT

The TON DHT is a distributed hash table used to discover peers and signed network values. It is required for autonomous network discovery beyond static global configs.

## Core Concepts

- DHT node id: public key based identity.
- Address list: peer UDP/TCP addresses with version and expiration.
- Key description: describes what value is stored.
- Value: bytes with TTL and signature.
- K-bucket-like peer lookup behavior.
- Reverse ping: helps validate reachability.

## Data Integrity

DHT values are signed. Implementation must verify:

- public key matches node id,
- signature covers the exact TL object required by the schema,
- TTL has not expired,
- address list version is current enough.

## Discovery Flow

Typical lookup:

1. Start from static DHT nodes from global config.
2. Query closest known peers for a key.
3. Validate returned nodes.
4. Continue until enough close peers or value is found.
5. Cache valid peers with expiration.

## Crate Mapping

`tonutils-tl::tl::network` contains checked ADNL/DHT node, key, value, and
lookup constructors. `tonutils-adnl::AdnlUdpSession::dht_find_node` sends a
query over authenticated UDP, decodes the boxed `dht.nodes` answer, and keeps
only nodes with usable addresses, non-expired versions, and valid Ed25519
signatures. `DiscoveryConfig::discover_typed` then applies deterministic
deduplication and explicit seed fallback.

`DiscoveryRecord` remains a compatibility abstraction for callers that already
have signed records. Full iterative Kademlia-style lookup, DHT value lookup,
and overlay-specific value validation remain above this boundary.

## Required Tests

- DHT node signature verification.
- Value signature verification.
- Expired value rejection.
- Closest-peer selection.
- Lookup convergence in simulated network.
