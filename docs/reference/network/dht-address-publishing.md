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
peer discovery from compounding: the missing value closes the path through
third parties, not delivery as a whole. Peers this node contacted itself keep
its routable source address from its datagrams
(`AdnlPeerPairImpl::receive_packet_checked`), and those peers do push
broadcasts to a non-publishing node - runs 38594880666 (38 inbound overlay
custom messages) and 38558546040 (4313) both delivered the external message
while `dht address publish: stored on 0 of 4` in every recorded pre-fix run,
and run 38885001661 delivered in a sample whose round stored on 0 nodes
(sample 2, `time_to_message=1.647s`) - delivering does not require the
stored value.

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
  (the `0x4813b4c6` `pub.ed25519` prefix plus the 32 key bytes), i.e. upstream's
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
- `dht.store` carries the value in the **bare** `dht.value` form: after the
  `0x34934212` query id the `key`, `value`, `ttl`, and `signature` fields follow
  directly, with no `0x90ad27cb` prefix (upstream `dht_store::store` calls
  `TlStoreObject::store(value_, s)`). `dht.valueFound` carries the same struct
  boxed. A query that sends the boxed form inside `dht.store` does not parse on
  the receiving node, which drops it silently and leaves the client with a
  timeout - the fault behind every recorded `stored on 0 of 4` run before the
  framing fix.
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
2. Candidate set: the known DHT nodes, plus `dht.findNode` rounds with
   `k = FIND_NODE_K` (8) toward the key id, merged and de-duplicated by short
   id. The known set is usually the static seed list, which is rarely the part
   of the keyspace a fresh node id lands in, and the closest seed of that list
   is not guaranteed to answer: the round asks the closest
   `ENRICH_ATTEMPTS` (3) seeds in distance order and stops at the first that
   answers, so one silent seed no longer degrades the round to the unenriched
   list (run 38885001661 lost the round on a silent closest seed in 6 of 8
   samples).
3. Sort candidates by XOR distance to the key id and send `dht.store` to the
   `PUBLISH_TARGETS` (4) closest concurrently. A node keeps a value only when
   it is close enough to the key: upstream `store_in` compares the distance
   against its bucket radius (`k + 10`) and drops "too remote" values, then
   runs `value.check()`, which verifies both signatures and the update rule -
   a malformed signature, like a mis-framed value (see Wire Format), produces
   no answer at all.
4. `verify_dht_address_value` asks the nodes that answered `dht.stored` -
   they are the nodes that hold the value - concurrently, and falls back to
   the closest known seeds only when the store failed everywhere. A
   `dht.valueNotFound` answer is followed through the closer nodes it carries,
   the same walk `resolve_address_on_session` makes, for up to
   `VERIFY_WALK_ROUNDS` (3) rounds of `VERIFY_WALK_BRANCH` (4) nodes, so a
   node that never received the store can still lead the walk to one that did.
   `dht.valueFound` plus an own-key match, `verify_signature()`, and the
   expected `ip:port` in the payload means the DHT serves the value
   (`served_by_dht=true` in the log line, which names the answering node and
   the round); a value that is found but does not match this node is a stale
   value and still reports `false`. A `dht.stored` answer alone only proves
   one node accepted the value. Before the verify asked the storing nodes
   directly, one recorded sample of run 38888038146 reported
   `served_by_dht=true` (sample 4: `stored on 4 node(s)`) only because the
   closest known seed happened to be a store target - the other samples of
   that run stored on 3-6 nodes and still reported `false`.

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
4. A STUN binding exchange (`udp_session/stun.rs`), reached only when the
   route probe reports a private address. An RFC 5389 `BindingRequest` is
   sent to three public servers and the `XOR-MAPPED-ADDRESS` of the
   `BindingSuccessResponse` is read back.

The probe cannot see a NAT boundary, and the binding exchange is what lets a
NAT'd node publish at all. Two details make the reported address the one
overlay peers can deliver to:

- The request is sent from the scanner's **shared ADNL transport socket**
  through `AdnlUdpTransport::raw_exchange`, not from a fresh probe socket.
  NAT maps each socket separately, so only the mapping of the socket the
  sessions actually talk on describes a port that answers.
- A mapping is published only when two servers report the same `ip:port` and
  the address is globally routable. Endpoint-independent mapping reports the
  same answer for every server; a NAT that maps per destination reports a
  different port per server and is rejected, since that port is reachable
  only from the server that observed it.

Limits and edge cases:

- An observed mapping is evidence about where the socket is reachable from
  the servers that answered, not proof that every overlay peer can deliver to
  it. The round logs its reasoning
  (`stun reports <ip:port> reachable for this socket`,
  `stun servers disagree`, `stun reached no agreement`) rather than asserting
  reachability, so a run on a NAT that filters unsolicited inbound traffic
  still reports what it observed.
- The exchange handles IPv4 only. A server that resolves to IPv6 alone is
  treated as unreachable, and the IPv6 form of `XOR-MAPPED-ADDRESS` is not
  decoded.
- The route probe observes a private source address and publishing
  self-skips instead of poisoning the value for its whole hour-long TTL when
  no address is configured and STUN reaches no agreement. This is the same
  reason upstream skips an address-less list.
- The published port is the one STUN reports for the shared transport socket
  when STUN engages, and the transport's bound port when the route probe
  succeeds. If neither can observe a reachable address, it must be configured
  explicitly through `external_address` or `TON_MEMPOOL_EXTERNAL_ADDRESS`.
- If nothing resolves to a globally routable address, the round does no
  stores at all and logs
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
  `DhtValueBare` (the bare `dht.store` framing), `DhtStored`,
  `AddressListBoxed`.
- `crates/tonutils-mempool/src/udp_session/stun.rs` - RFC 5389 binding client:
  `binding_request`, `parse_binding_response`, and `discover_mapped_address`.
- `crates/tonutils-adnl` - `AdnlUdpTransport::for_node` (the shared socket the
  probe reports the bound port from and the stores send through) and
  `AdnlUdpTransport::raw_exchange` (sends the binding request from that socket
  and captures its reply, which the demultiplexer would otherwise drop),
  `AdnlUdpSession::dht_store` (frames the query over `DhtValueBare`),
  `KeyPair::sign_raw`, `now_i32`, `local_reinit_date`.

## Tests

- Unit tests in `publish.rs`: `address_value_is_signed_and_parses_back`
  (both signatures verify, key/name/idx, payload, one-hour ttl),
  `address_value_supports_ipv6`, `xor_distance_orders_closest_first`,
  `route_probe_rejects_non_routable_addresses`, and
  `candidates_parse_from_seed_peers`.
- Unit tests in `stun.rs`: `request_carries_header_and_transaction_id`,
  `xor_mapped_address_is_decoded`, `legacy_mapped_address_is_decoded_without_xor`,
  `xor_form_wins_over_the_legacy_attribute`,
  `padded_attributes_are_walked_without_bleeding_into_the_next`,
  `response_for_another_transaction_is_rejected`, `truncated_response_is_rejected`,
  `attribute_past_the_declared_length_is_rejected`, and
  `non_ipv4_address_attributes_are_ignored`. Responses are encoded field by
  field from RFC 5389 rather than captured from a live server, so the parser is
  checked against the specification.
- Unit test in `tonutils-adnl`: `raw_exchange_returns_reply_and_recovers_from_timeout`
  (`adnl/udp/tests/`) covers reply capture on the shared socket and hook
  cleanup after a timeout.
- Unit test in `tonutils-tl` (`network.rs`): `dht_store_carries_the_bare_value`
  pins the store framing - the query is the `0x34934212` id followed by the
  bare `dht.value` fields, the boxed `DhtValue` form is exactly those fields
  behind the `0x90ad27cb` prefix, and the query round-trips. Upstream's
  generated `dht_store::store` serializes the field the same way, so the test
  fails on the boxed framing that dropped every store on the wire before the
  fix.
- Live gate: `configured_seed_delivers_valid_external_message` in
  `crates/tonutils-mempool/tests/live.rs`, run by
  `.github/workflows/live-tests.yml` on a public-IP runner where probing and
  publishing engage. The acceptance criterion remains `overlay_packets > 0`
  in `TODO.md`.

## Missing Work

- The QUIC path (`native_quic`) does not publish an `address` value; mainnet
  seeds expose no QUIC endpoint, so discovery there is deferred in `TODO.md`.
- First runner-observed stores reached the public DHT in run 38885001661
  (2026-10-16): `stored on 4 node(s)` and `stored on 3 node(s)` in 2 of 8
  samples with `task_panics=0` everywhere, after the `dht.store` framing fix.
  Run 38888038146 recorded the first `served_by_dht=true` (sample 4, `stored
  on 4 node(s)`) - a single occurrence that depended on the closest known seed
  being a store target, not the answer the code now asks the storing nodes
  for. Third-party resolve-and-ping of this node remains unobserved, and a
  NAT'd local host cannot produce that confirmation: the local upstream
  `dht-server` rig only proves the round itself (`stored on 1 of 1`,
  `served_by_dht=true` through the new targets-first verify).
- There is no SOCKS5 UDP relay yet, so a NAT'd host that must publish cannot
  do so without configuring `external_address` explicitly.
