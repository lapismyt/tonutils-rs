# QUIC TLS and wire profile

## Purpose and scope

This page specifies the handshake and stream framing used by the `quic`
feature of `tonutils-adnl`: TLS version pinning, ALPN, SNI derivation, raw
public key certificates in both directions, and signature verification.
It records upstream `ton` facts (`quic/*.cpp`, `quic/*.h`) next to the
rustls mechanisms that implement them, plus the local and cross-SDK
evidence behind each claim.

Discovery targets and the live reachability question live in
[Overlay protocol design](overlay.md#quic-member-discovery); the claim
summary lives in the [protocol matrix](protocol-matrix.md).

## Transport and framing

- `quinn` 0.11 over UDP with the `ring` TLS provider; `QuicSession::connect`
  for clients, `QuicServer::{bind,accept}` for servers.
- Upstream full nodes listen on `adnl_udp_port + 1000`
  (`quic/quic-sender.h::NODE_PORT_OFFSET`); an advertised
  `adnl.address.quic` record wins over the derived port.
- Queries and answers use the upstream `ton_api.tl` framings
  (`quic.query data:bytes = quic.Request`, `quic.answer data:bytes =
  quic.Response`, `quic.message data:bytes = quic.Message`) on bidirectional
  streams. Correlation is per stream; the frames carry no query id, and the
  answer payload must decode as the expected constructor or the query fails
  closed with `AdnlError::InvalidQuicAnswer`.
- Inbound frames are capped at 1 MiB (`MAX_FRAME_SIZE`).

## TLS profile (wire facts)

| Fact | Value |
| --- | --- |
| Protocol version | TLS 1.3 only (`rustls::version::TLS13`, mirroring upstream `SSL_CTX_set_min/max_proto_version(TLS1_3)` in `quic/quic-pimpl.cpp::setup_rpk_context`) |
| ALPN | `ton` (upstream `QuicServer`/`QuicSender` `alpn = "ton"`) |
| SNI | `<hex[0:32]>.<hex[32:64]>.adnl` - lowercase hex of the peer's ADNL short id, split at 32 characters |
| Short id | SHA-256 over the TL-tagged Ed25519 public key (`pub.ed25519`, constructor id `0xc6b41348`), i.e. `AdnlAddress::from(&PublicKey)` |
| Certificate payload | RFC 7250 raw public key: a 44-byte Ed25519 SubjectPublicKeyInfo (`302a300506032b6570032100` prefix, RFC 8410) or a bare 32-byte key. No X.509 chain, no SAN, no fallback |
| Client certificate | Required. The server offers client auth, marks it mandatory, and requires the raw-public-key certificate type; upstream requests a certificate (`SSL_VERIFY_PEER`) and drops a missing or non-32-byte peer key in `quic/quic-pimpl.cpp::parse_peer_id` |
| CertificateVerify | Standard RFC 8032 Ed25519 over the TLS 1.3 transcript hash, verified with `PublicKey::verify_raw`. TLS 1.2 signature verification returns `Err` (fail-closed) |

### SNI gating

Upstream selects the server identity from the ClientHello name
(`ServerIdentity::sni`, `quic/quic-common.h`) and fails the handshake with a
fatal `unrecognized_name` alert for an unknown name (`quic/quic-pimpl.cpp`
SNI callback), while a client that sends no SNI keeps the default identity.
Our rustls path reproduces both branches in `SniRpkResolver`, the
certificate resolver being the only `ClientHello`-time hook rustls exposes:

- name equals the node's short-id SNI (exact, case-sensitive comparison -
  upstream looks the name up in a map keyed by lowercase hex): identity
  resolves and the handshake continues;
- name differs: no certificate resolves, rustls sends the fatal
  `no server certificate chain resolved` alert (`AccessDenied`), and both
  endpoints observe a failed handshake;
- name absent: the identity resolves, mirroring OpenSSL's default identity.

A fixed vector pins the derivation: seed `[0x42; 32]` yields public key
`2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12`
(verified with `openssl pkey -inform der -pubout`), short id
`c46870c7c81b3b56bafd6e0836362a189ebc58281029effb8460bda50f3a3af0`
(verified with `sha256` over `c6b41348 || pubkey`), SNI
`c46870c7c81b3b56bafd6e0836362a18.9ebc58281029effb8460bda50f3a3af0.adnl`.

### Certificate-type interlock

RFC 7250 lets each side offer the certificate types it understands; the
peer's selection travels in the TLS 1.3 EncryptedExtensions (RFC 7250
ss 4.2 places the selection in the server hello, and EncryptedExtensions is
its TLS 1.3 carrier). rustls implements this as:

- `requires_raw_public_keys() = true` on the client verifier makes the
  client offer `server_certificate_types = [raw_public_key]` in its
  ClientHello and require the server's selection in EncryptedExtensions;
- `requires_raw_public_keys() = true` on the server's client verifier makes
  the server require `client_certificate_types = [raw_public_key]` in the
  ClientHello before it accepts the handshake;
- the raw-public-key resolvers (`AlwaysResolvesClientRawPublicKeys`,
  `AlwaysResolvesServerRawPublicKeys`, wrapped by `SniRpkResolver` on the
  server) make each side present its identity key as the certificate.

Consequences, each covered by a loopback test: a client that offers no raw
public key cannot complete the handshake; a client whose server key differs
from the dialed key fails in `verify_server_cert` with
`AdnlError::CertificateKeyMismatch`; a client whose SNI is not the server's
short id fails during the handshake.

## Upstream fact mapping

| Upstream (`ton-blockchain/ton`) | This crate |
| --- | --- |
| `setup_rpk_context`: TLS 1.3 only, `SSL_CTX_set1_server/client_cert_type(TLSEXT_cert_type_rpk)` | `make_quinn_{client,server}_config`: `with_protocol_versions(&[TLS13])`, `requires_raw_public_keys() = true`, `AlwaysResolves{Client,Server}RawPublicKeys` |
| `alpn = "ton"` | `ALPN_TON` on both configs |
| `ServerIdentity::sni` (short id, split 32/32, `.adnl`) | `sni_for_public_key` |
| SNI callback: fatal `unrecognized_name` for an unknown name, default identity otherwise | `SniRpkResolver::resolve` |
| `SSL_VERIFY_PEER` plus `parse_peer_id` rejecting a missing peer key | `TonRpkClientVerifier` (`offer_client_auth`, `client_auth_mandatory`, `verify_client_cert`) |
| Peer key from `SSL_get0_peer_rpk` (32-byte raw public key) | `extract_ed25519_key` (44-byte SPKI or 32-byte key) |
| Certificate signature verified as Ed25519 over the transcript | `verify_ed25519_signature` via `PublicKey::verify_raw` |

## Crate mapping

- `crates/tonutils-adnl/src/adnl/quic_tls.rs` (private module): SNI
  derivation, key extraction, `TonServerCertVerifier`, `TonRpkClientVerifier`,
  `SniRpkResolver`, `make_quinn_client_config`, `make_quinn_server_config`.
- `crates/tonutils-adnl/src/adnl/quic.rs`: `QuicSession` (connect, query,
  message, DHT/overlay helpers) and `QuicServer` (bind, accept, reported
  SNI), plus loopback and negative-handshake tests.
- `crates/tonutils-adnl/src/adnl/crypto.rs`: `sign_raw`/`verify_raw`
  (standard Ed25519) and `KeyPair::private_key_der`, which embeds the
  original 32-byte seed so `rustls` derives exactly the public key the pair
  carries.
- `crates/tonutils-mempool/src/quic_session.rs` and `endpoints.rs`: dial
  targets for overlay members (see [overlay.md](overlay.md)).

## Evidence

- Unit tests in `quic_tls.rs`: the SNI fixed vector above, both certificate
  encodings plus foreign encodings (wrong length, foreign SPKI prefix,
  non-point y coordinate, X.509 container), verifier pinning and
  `requires_raw_public_keys`/`offer_client_auth`/`client_auth_mandatory`
  flags, standard Ed25519 `CertificateVerify` acceptance with tampering,
  wrong-scheme, wrong-key, and truncation rejection, and a binding check
  that `rustls` signs with the pair's own identity key.
- Loopback tests in `quic.rs`: query/answer exchange with the reported
  short-id SNI, `InvalidQuicAnswer` fail-closed behavior, and three
  negative handshakes (mismatched server key, client without a raw public
  key, unrecognized SNI).
- Cross-SDK interop (local, 2026-10-12) against upstream's
  `test/quic-interop-rust` reference pair with its ALPN patched from
  `hq-interop` to `ton`: our `QuicSession` completed the handshake against
  `rpk-server` and received an HTTP 200, and upstream's `rpk-client`
  completed our mandatory-client-auth, SNI-gated handshake and finished all
  of its phases (path migration, CID rotation) with exit code 0.
- Live reachability (CI runs 38237835993, 38289674977, and
  38292378027, 2026-10-12): mainnet QUIC ports are
  mostly silent to third-party sources from GitHub
  Actions, but the fleet is not uniformly unreachable.
  Run 38237835993 measured `vn_replies=0` from 13
  member targets while both controls answered. Run
  38289674977 refined this over 25 targets: 24 stayed
  completely silent (no Version Negotiation reply, no
  handshake response), while one member
  (`198.13.134.211:9589`) runs a live QUIC endpoint
  that actively aborts the cryptographic handshake with
  CRYPTO_ERROR `0x170` - TLS alert 112 =
  `unrecognized_name` (RFC 6066; `bad_certificate_status_response`
  is alert 113, `0x171`). That is exactly the alert
  upstream's SNI callback sends for an unknown name (see
  "SNI gating" above), so the rejection is consistent with
  dialing a guessed `udp+1000` port whose QUIC listener
  hosts a different node identity than the dialed member.
  Run 38292378027 measured the first
  completed handshake: 23 targets, 21 silent, 1
  aborting with alert 112 (`91.108.10.44:33000`), and
  1 (`5.199.161.213:31310`) completing the full TLS 1.3
  / RFC 7250 raw-public-key handshake with mandatory
  client auth - so the live test's hard `connected > 0`
  assert passed and the CI step went green. The
  connected member did not answer `overlay.getRandomPeers`
  within 15s (logged as `query_error`) - the symptom whose root
  cause was later confirmed and fixed (see the client-side
  addressing gap below, tracked in `TODO.md`). The live test's
  per-target `phase=connect` lines separate silence,
  active rejection, and completed handshakes on every
  run.
- Live reachability, 2026-10-14/15 (CI runs 38649670525 and
  38699045977 plus its re-run; the test now lives in
  `crates/tonutils-mempool/tests/live_quic.rs`): run 38649670525
  probed 24 targets and went green with
  `connected=2 timeouts=19 errors=3` - both connected targets
  (`88.99.250.112:31303`, `62.210.145.26:50769`) were
  `source=udp+1000` (the phase-2 `quic probe:` lines say so)
  and both then logged `query_error=QUIC query timed out after
  15s`, while 3 targets aborted with alert 112. Run 38699045977
  recorded `connected=0 timeouts=16 errors=4` over 20 targets
  (alert 112 from `162.43.189.13:61912`, `85.90.235.131:31001`,
  `91.108.10.7:33000`, `51.222.42.108:24633`) and its re-run
  recorded `connected=0` over 22 targets with 2 alert-112
  rejections - a false red for PR #65. The only advertised
  endpoint dialed in that run (`91.134.9.14:4286`,
  `source=advertised`) timed out. Three conclusions:

  - (a) alert-112 rejections have mostly been on guessed
    `udp+1000` ports: three of the four in run 38699045977
    carry an explicit `quic probe: ... source=udp+1000` line, and
    the fourth (`51.222.42.108:24633`) was confirmed
    `source=udp+1000` from its member's address record; many
    members share hosting IPs (5 members on
    `198.244.253.169`, 3 on `173.234.75.230` in that run), so the
    listener behind the guessed port selects its own identity from
    the SNI instead of the dialed member's. The counterexample
    landed in run 38723511984 (2026-10-15): `141.94.3.10:4279`
    aborted with alert 112 as `source=advertised`. The mechanism
    is unchanged - the listener rejects a dial whose SNI name it
    does not know - but the guessed-port-only qualifier no longer
    holds, so
    an advertised `adnl.address.quic` record can itself be stale
    or point at a foreign listener (see the live-confirmation
    bullet below).
  - (b) guessed ports do complete handshakes when the member runs
    QUIC on `udp+1000` without advertising it - both connected
    targets of the green run were `source=udp+1000` - so growing
    the candidate pool still helps.
  - (c) the post-handshake silence is systematic: all four
    handshakes that completed across the recorded runs
    (`5.199.161.213:31310` in 38292378027, `18.183.41.221:32000`
    in 38594880666, `88.99.250.112:31303` and
    `62.210.145.26:50769` in 38649670525) never answered
    `overlay.getRandomPeers` within 15s (4/4 `query_error`) - the
    silence was traced to the missing `overlay.query{overlay}` header
    on our QUIC client and fixed on 2026-10-15 (see the client-side
    addressing gap below); live confirmation landed the same day in
    run 38723511984 (see below).
- Reference-SDK gap (verified 2026-10-15 from the GitHub source
  trees): pytoniq and pytoniq-core contain no QUIC client at all -
  their ADNL transport is UDP-only - so no SDK reference exists to
  compare our ADNL-over-QUIC query pattern with; the comparison has
  to come from upstream C++ directly.
- Upstream query dispatch over QUIC (verified 2026-10-15 against
  `ton-blockchain/ton` master): `quic/quic-sender.cpp::
  QuicSender::on_inbound_query` takes `quic.query.data` and asks
  `AdnlPeerTable::deliver_query(src = connection->path.second`
  (the remote peer), `dst = connection->path.first` (the local
  node id the server was bound to through `QuicSender::add_id`),
  `data)`; `adnl/adnl-peer-table.cpp::AdnlPeerTableImpl::
  deliver_query` resolves `dst` in `local_ids_` and forwards to
  `adnl/adnl-local-id.cpp::AdnlLocalId::deliver_query`, which
  dispatches on a 4-byte TL-prefix match against the callbacks
  subscribed on that node id. There is no `adnl.message.query`
  wrapper in this path - `quic.query.data` must begin with the
  inner query's own TL constructor (or with the `overlay.query`
  header, below). When no prefix matches, `AdnlLocalId::
  deliver_query` rejects with "no callbacks for custom query",
  and `QuicSender::on_inbound_query` sends `quic.answer` only
  along its success path (it has no error branch), so the stream
  is left unanswered and the client observes a query timeout.
- How upstream serves `overlay.getRandomPeers` (research
  2026-10-15, `overlay/overlay-manager.cpp`): the manager
  subscribes exactly four TL prefixes on the member's NODE ADNL
  id - `overlay.message` (`0x75252420`), `overlay.query`
  (`0xccfd8443`), `overlay.messageWithExtra` (`0xa232233d`), and
  `overlay.queryWithExtra` (`0x94ffc3e9`) - in
  `OverlayManager::register_overlay(local_id, overlay_id, ...)`,
  and callers pass their own node id as `local_id` (for example
  `validator/full-node-fast-sync-overlays.cpp::
  create_semiprivate_overlay(local_id_, ...)`, the same
  `local_id_` it registers on `QuicSender::add_id`). So an
  overlay query arriving on the member's NODE id is answerable;
  the overlay id is never an ADNL destination in this chain, it
  travels inside the `overlay.query` header. After the prefix
  match, `OverlayManager::receive_query` strips that header with
  `fetch_tl_prefix<overlay_query(WithExtra)>`, resolves
  `overlays_[dst][overlay_id]` (node id first, then overlay id -
  an overlay the node did not join under that id is buffered or
  rejected with `bad local_id` / `bad overlay_id`), and hands the
  bare inner query to `OverlayImpl::receive_query`, whose
  `process_query(overlay_getRandomPeers&)` answers with
  `overlay.nodes`. Upstream builds the outgoing header in
  `OverlayManager::send_query_via` with
  `create_serialize_tl_object_suffix<ton_api::overlay_query>
  (query.as_slice(), overlay_id.tl())` - a 4-byte constructor,
  the 32-byte overlay id, then the raw inner query bytes - on
  every transport, QUIC included.
- Client-side addressing gap - root cause confirmed and fixed
  (2026-10-15): historically `QuicSession::overlay_get_random_peers`
  (`crates/tonutils-adnl/src/adnl/quic.rs`) ignored its `_overlay`
  argument and serialized only `overlay.getRandomPeers`
  (`0x48ee64ab`) as `quic.query.data`, while the UDP client
  (`AdnlUdpSession::send_overlay_get_random_peers_with_id`)
  prefixes `overlay.query{overlay}` exactly like upstream. Against
  an upstream node the bare query matches no subscribed prefix, so
  it is dropped with "no callbacks for custom query" and no
  `quic.answer` is sent - which explains the 4/4 post-handshake
  timeouts above. Nothing local could falsify it either: no test
  exchanged `overlay.getRandomPeers` over QUIC
  (`QuicOverlaySession::receive` finished inbound `quic.query`
  streams without a `quic.answer`), and the UDP-side dispatcher
  (`overlay_inbound::build_overlay_answer`) accepts the bare
  constructor as well as the wrapped one, so our own client and
  server agreed on a shape upstream rejects. The root cause is now
  confirmed against upstream `ton_api.tl` (`overlay.query
  overlay:int256 = True;` - a 4-byte constructor and the 32-byte
  overlay id with the inner query appended raw, no `bytes` length
  prefix) and the `overlay/overlay.cpp` dispatch chain documented
  above. The fix makes `QuicSession::overlay_get_random_peers` build
  `overlay.query{overlay} ++ overlay.getRandomPeers` exactly like
  `send_overlay_get_random_peers_with_id` and upstream's
  `OverlayManager::send_query_via`; the loopback test
  `overlay_get_random_peers_wraps_the_overlay_query_header`
  (`crates/tonutils-adnl/src/adnl/quic.rs`) asserts byte equality
  of the whole payload, and `OverlayImpl::process_query` answers an
  empty `peers:overlay.nodes` list regardless (`add_peers` of an
  empty list is a no-op), so the header-only payload needs no
  signed local node record - the only cost is peer admission, since
  an empty list does not queue this node in the member's
  `pending_peers_` (`docs/reference/network/overlay.md`), leaving
  ping-back visibility to the UDP path that carries the signed
  record. Live confirmation landed in run 38723511984 (the next
  CI live run - see the bullet below). Side benefit:
  `QuicOverlaySession::receive`'s 5s
  keepalive (`crates/tonutils-mempool/src/quic_session.rs`) calls
  the same method with a 3s budget and previously always burned
  that full timeout on a never-answered query, stalling the receive
  loop; with answers returning it completes in about one RTT.
- Live confirmation, 2026-10-15 (CI run 38723511984, `694aa49`): the
  first live run after the wrapper fix. The sole connected target
  (`88.198.126.194:57996`, `source=udp+1000`, wave 1) answered the
  wrapped query with four nodes - logged as
  `phase=connect target=88.198.126.194:57996 source=udp+1000 wave=1
  result=connected random_peers=4` - the first live answer to
  `overlay.getRandomPeers` over QUIC, ending the 4/4 post-handshake
  silence streak and live-confirming the `overlay.query{overlay}` fix.
  The stabilization also held: `phase=members registered=17 udp_peers=17`
  then `phase=members-round2 registered=33 udp_peers=17` (the second
  discovery round nearly doubled the pool), 33 targets dialed,
  `phase=connect targets=33 connected=1 timeouts=29 errors=3
  connected_advertised=0 connected_udp_offset=1 waves=1`, and the step
  went green on wave 1 (early stop at the first handshake);
  `phase=version-negotiation targets=33 vn_replies=0 controls=
  1.1.1.1:443:reply,8.8.8.8:443:reply` confirms the reachability
  verdict is unchanged for the 5 s probe budget - `phase=probe
  members_before=33 quic_peers=0 members_after=33`, so the probe phase
  still finds nothing and only the 10 s wave-1 connects answer. The
  alert-112 nuance: this run produced the first alert 112 on an
  `advertised` endpoint (`141.94.3.10:4279`, `source=advertised`), and
  the run's other two 112s were `udp+1000` (`84.207.214.244:21470`,
  `85.90.235.130:31005`) - the mechanism (SNI `unrecognized_name`)
  holds, but a guessed-port-only qualifier no longer does: an
  advertised `adnl.address.quic` record can itself be stale or point
  at a foreign listener. The run's second advertised target
  (`47.242.120.137:31001`) timed out.

## Missing work

- A quictls/C++ client against our mandatory-client-auth server is the one
  pairing upstream's `test-quic-interop.sh` matrix never exercised (their
  Test 3 rustls server ran with `offer_client_auth() = false`); tracked in
  `TODO.md`.
- Whether mainnet full nodes answer QUIC from third-party sources
  has a three-way verdict across the recorded runs: most probed
  members are silent, some abort the handshake with TLS alert 112
  (`unrecognized_name`, an SNI identity mismatch - the listener
  serving a different node identity than the dialed name, including
  the first one on an `advertised` endpoint in run 38723511984, see
  Evidence above), and four handshakes completed in the pre-fix
  runs (one each in runs
  38292378027 and 38594880666, two in run 38649670525; all four
  were dialed as `adnl_udp_port + 1000`, two of them with an
  explicit `source=udp+1000` probe line) without any of them
  answering `overlay.getRandomPeers` within 15s - 4/4
  systematic silence explained by the
  missing `overlay.query` header on our QUIC client, confirmed
  against `ton_api.tl` and the upstream dispatch chain, fixed
  in `QuicSession::overlay_get_random_peers` with the loopback
  test named above (Evidence above, root-cause item in
  `TODO.md`) and live-confirmed in run 38723511984 (2026-10-15),
  where the connected member answered the wrapped query with
  `random_peers=4` (the closing `TODO.md` item). The test
  behind that gate was stabilized for it - two discovery rounds
  against one registry (`max_records=32`, a second round below
  40 members), a 10 s per-target connect budget, up to three
  retry waves that redial only failed targets,
  `source=`/`wave=` on every per-target line, and
  `connected_advertised=`/`connected_udp_offset=`/`waves=`
  counters in the summary (moved to `tests/live_quic.rs`, CI step
  now runs `--test live_quic`) - while the hard `connected > 0`
  gate is kept by project decision, so the step is green in the
  runs where at least one member completes a handshake. The
  wrapper mechanism is now live-verified (run 38723511984), so the
  outstanding unproven pairing next to it is the C++-client
  pairing above.
- Concurrent routing and unsolicited-message handling on `QuicSession` are
  not implemented (see the [protocol matrix](protocol-matrix.md)).
