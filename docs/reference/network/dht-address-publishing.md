# DHT address publishing

## Purpose And Scope

The DHT `address` value is the record that lets any other node resolve this
node's reachable UDP address from its ADNL id alone. Upstream publishes it
whenever the local id's address list changes (`AdnlLocalId::publish_address_list`
in `adnl/adnl-local-id.cpp`), keyed by

```
dht.key id:<pubkey hash> name:"address" idx:0
```

and pytoniq's client stores the same value with `dht.store`. A node that never
publishes it can only be reached by peers it contacted itself: every peer that
learns its signed `overlay.node` record from an `overlay.getRandomPeers` answer
fails the `dht.getValue(address)` resolution, cannot `overlay.ping` it, and
therefore never adds it as a verified neighbour. That is what stops transitive
peer discovery from compounding, and why no broadcast is ever pushed to a
non-publishing node.

This page covers value construction, the publish/verify round, and
external-address resolution for the native UDP path (`native_udp` and
`native_udp_seeds_only` in `tonutils-mempool`). Consuming the value - address
resolution of other candidates - is covered by [TON DHT](dht.md) and
[Mempool scanner design](../mempool/scanner.md). The QUIC path does not publish
yet; see Missing work.

## Wire Format

| TL constructor | id |
|---|---|
| `dht.key id:int256 name:bytes idx:int = dht.Key;` | `0xf667de8f` (boxed) |
| `dht.keyDescription key:dht.key id:PublicKey update_rule:dht.UpdateRule signature:bytes = dht.KeyDescription;` | `0x281d4e05` (boxed) |
| `dht.updateRule.signature = dht.UpdateRule;` | `0xcc9f31f7` |
| `dht.value key:dht.keyDescription value:bytes ttl:int signature:bytes = dht.Value;` | `0x90ad27cb` |
| `dht.store value:dht.value = dht.Stored;` | `0x34934212` |
| `dht.stored = dht.Stored;` | `0x7026fb08` |
| `dht.findNode key:int256 k:int = dht.Nodes;` | `0x6ce2ce6b` |
| `dht.findValue key:int256 k:int = dht.ValueResult;` | `0xae4b6011` |
| `adnl.addressList addrs:(vector adnl.Address) version:int reinit_date:int priority:int expire_at:int = adnl.AddressList;` | `0x2227e658` |
| `adnl.address.udp ip:int port:int = adnl.Address;` | `0x670da6e7` |

- `dht.key.id` is the node's ADNL short id: sha256 of the boxed public key
  (the `0xc6b41348` `pub.ed25519` prefix plus the 32 key bytes), i.e. upstream's
  `short_id_.pubkey_hash()`. `name` is the literal bytes `"address"`, `idx` is
  `0`.
- The routing key id used by `dht.findNode`/`dht.findValue` is sha256 of the
  *boxed* `dht.key` serialization (constructor prefix included), which is what
  `dht_key_id` computes for both publishing and lookup.
- Both signatures cover the **boxed** serialization with the `signature` field
  still empty, matching upstream: the description signature covers `0x281d4e05`
  plus fields with `signature = []`, and the value signature covers `0x90ad27cb`
  plus fields with `signature = []` (`DhtKeyDescription::unsigned_bytes`,
  `DhtValue::unsigned_bytes`). The update rule is `dht.updateRule.signature`,
  so only the key owner may replace the value.
- `value` is the boxed `adnl.addressList` serialization: one
  `adnl.address.udp` entry (IPv4 word stored as a signed little-endian `int`,
  i.e. `u32 as i32`; `port` as `int`) or one `adnl.address.udp6` entry for
  IPv6. `version` is `now` at build time, `reinit_date` is this process's ADNL
  reinit date (`local_reinit_date`, fixed for the process lifetime),
  `priority = 0`, `expire_at = 0`.
- `ttl = now + 3600`. Upstream `DhtMemberImpl::store_in` rejects a value whose
  `ttl` is greater than `now + 3600 + 60` ("ttl is too big"), so one hour is
  the maximum useful lifetime; `PUBLISH_INTERVAL` of 10 minutes re-publishes
  well inside it.

## Publish Round

`publish_dht_address_value` runs one round:

1. Build and sign the value for the resolved external address (below).
2. Candidate set: the known DHT nodes, plus one `dht.findNode` round with
   `k = FIND_NODE_K` (8) toward the key id, merged and de-duplicated by short
   id. The known set is usually the static seed list, which is rarely the part
   of the keyspace a fresh node id lands in.
3. Sort candidates by XOR distance to the key id and send `dht.store` to the
   `PUBLISH_TARGETS` (4) closest concurrently. A node keeps a value only when
   it is close enough to the key: upstream `store_in` compares the distance
   against its bucket radius (`k + 10`) and drops "too remote" values, then
   runs `value.check()`, which verifies both signatures and the update rule -
   a malformed signature produces a query error instead of `dht.stored`.
4. `verify_dht_address_value` asks the closest known node with `dht.findValue`.
   `dht.valueFound` plus an own-key match, `verify_signature()`, and the
   expected `ip:port` in the payload means the DHT serves the value
   (`served_by_dht=true` in the log line); `dht.valueNotFound` means the round
   has not taken effect yet. A `dht.stored` answer alone only proves one node
   accepted the value.

The publishing task runs one round before the first peer-growth round, so
transitive discovery can resolve this node's address right away, and then
every `PUBLISH_INTERVAL`.

## External Address Resolution And NAT Limits

The external address is resolved lazily on every round, first match wins:

1. `MempoolScannerBuilder::external_address`.
2. The `TON_MEMPOOL_EXTERNAL_ADDRESS` environment variable (`IP:PORT`).
3. A route probe: for each known DHT node address, a fresh UDP socket
   `connect()`s without sending a datagram, so the operating system picks the
   source address of the default route with no side effects. The source
   address is combined with the bound port of this node's shared transport
   socket. Only globally routable addresses are accepted
   (`is_globally_routable` rejects private, loopback, link-local, multicast,
   broadcast, documentation, unspecified, and IPv6 unique-local/unicast
   link-local addresses).

Limits and edge cases:

- On a NAT'd host the probe observes a private source address, so publishing
  self-skips with a debug log instead of poisoning the value for its whole
  hour-long TTL. CI runners and servers with public egress addresses publish;
  local development behind NAT deliberately does not. This is the same reason
  upstream skips an address-less list.
- The published port is the shared transport's bound port. If a NAT remaps
  UDP ports, the reachable address must be configured explicitly through
  `external_address` or `TON_MEMPOOL_EXTERNAL_ADDRESS`.
- If no probe address is globally routable and nothing is configured, the
  round does no stores at all and logs
  `no externally reachable address for this node`.

## Crate Mapping

- `crates/tonutils-mempool/src/udp_session/publish.rs` - value construction
  (`address_dht_value`), route probe (`detect_external_udp_address`,
  `is_globally_routable`), publish round (`publish_dht_address_value`),
  verification (`verify_dht_address_value`), and the `address_publisher`
  closure installed on the builder.
- `crates/tonutils-mempool/src/builder.rs` - `native_udp` and
  `native_udp_seeds_only` install the publisher; the periodic task publishes
  before the first peer-growth round and then every `PUBLISH_INTERVAL`, using
  the current known DHT nodes as candidates.
- `crates/tonutils-mempool/src/udp_session/lookup.rs` - `dht_key_id` (boxed
  key hashing), `shared_session` (one transport session per peer), and the
  consumer side `resolve_address`.
- `crates/tonutils-tl/src/tl/network.rs` - `DhtKey::boxed_bytes`,
  `DhtKeyDescription::unsigned_bytes`, `DhtValue::unsigned_bytes`,
  `DhtStored`, `AddressListBoxed`.
- `crates/tonutils-adnl` - `AdnlUdpTransport::for_node` (the shared socket the
  probe reports the bound port from and the stores send through),
  `KeyPair::sign_raw`, `now_i32`, `local_reinit_date`.

## Tests

- Unit tests in `publish.rs`: `address_value_is_signed_and_parses_back`
  (both signatures verify, key/name/idx, payload, one-hour ttl),
  `address_value_supports_ipv6`, `xor_distance_orders_closest_first`,
  `route_probe_rejects_non_routable_addresses`, and
  `candidates_parse_from_seed_peers`.
- Live gate: `configured_seed_delivers_valid_external_message` in
  `crates/tonutils-mempool/tests/live.rs`, run by
  `.github/workflows/live-tests.yml` on a public-IP runner where probing and
  publishing engage. The acceptance criterion remains `overlay_packets > 0`
  in `TODO.md`.

## Missing Work

- The QUIC path (`native_quic`) does not publish an `address` value; mainnet
  seeds expose no QUIC endpoint, so discovery there is deferred in `TODO.md`.
- Live confirmation that the value is served (`served_by_dht=true`) and that
  third parties resolve and ping this node through it is pending the CI run;
  it cannot be produced from a NAT'd local host by design.
- There is no SOCKS5 UDP relay yet, so a NAT'd host that must publish cannot
  do so without configuring `external_address` explicitly.
