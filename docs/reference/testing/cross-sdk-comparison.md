# Cross-SDK Byte Comparison

This page documents the offline mechanism that detects DHT, overlay, ADNL,
and QUIC wire-format divergence between `tonutils-rs` and reference SDKs:
the same logical input is serialized independently by each implementation
and the resulting bytes are compared byte-for-byte.

## Purpose And Scope

Roundtrip tests are self-inverse: an SDK that parses and re-serializes with
the same (possibly wrong) field layout still passes them. Divergence
against other implementations is only visible when identical inputs are
serialized independently and the outputs are compared. This mechanism does
that for the `dht.*`, `overlay.*`, `adnl.message.*`, and `quic.*`
constructor surface.

In scope: wire layout and constructor ids of those families.

Out of scope: semantic relations between fields (signatures, derived ids),
transport behavior, and liveness. Live-network evidence enters only as
recorded raw bytes (see Live Capture below).

## Fixture Format

Fixtures live in `fixtures/cross_sdk/`; `manifest.json` records the schema
revision, the comparison rule, reference pins, and one entry per file with
`evidence_kind`, capture date, and SHA-256. Each case carries:

- `name`, `constructor`: case identity and boxed constructor name,
- `references`: reference SDKs that must reproduce `raw_hex` from
  `fields`; empty for live captures,
- `fields`: language-neutral canonical input (offline cases only),
- `raw_hex`: the reference wire bytes,
- optional `note` for cases a reference cannot cover.

Field conventions, shared by all three implementations:

- `int256` and `bytes` values: lowercase hex strings,
- `int` and `long` values: JSON numbers,
- IPv4 addresses: dotted strings; each side converts to the wire int,
  which is the global-config style big-endian value (`127.0.0.1` =
  `0x7f000001`) written little-endian on the wire; pytoniq encodes the
  same value as signed `i32`, so addresses with a high first octet become
  negative there,
- `PublicKey`: plain 32-byte ed25519 hex,
- addresses: `{"udp": {"ip", "port"}}` or `{"quic": {"ip", "port"}}`,
- `update_rule`: `signature` | `anybody` | `overlayNodes`.

## References And Pins

| Reference | Pin | Generator | Covers |
| --- | --- | --- | --- |
| tonutils-go | `v1.18.0` (`749603ab237d058cac5be4c42d36a52064db8b58`) | `scripts/cross_sdk/gen_go` | all offline cases |
| pytoniq-core | `0.2.0` (PyPI, bundled `ton_api.tl` SHA-256 `db5f20a5…`) | `scripts/cross_sdk/gen_py.py` | cases without `quic.*`, `overlay.ping`, or `adnl.address.quic` |

Known reference constraints, encoded in `references`/`note` fields:

- pytoniq-core's bundled schema has no `quic.*`, `overlay.ping`, or
  `adnl.address.quic` constructors; those cases are tonutils-go-only and
  are additionally covered against the pinned upstream schema by
  `schema_audit.rs`,
- `gen_go` registers `quic.*` from the verbatim upstream schema strings
  instead of importing tonutils-go's `adnl/quic` package: the pinned
  `quic-go-ton` transport fork does not compile under Go 1.27 and is
  irrelevant to TL wire layout,
- `gen_go` registers plain stand-in structs for `adnl.message.query` /
  `answer` because tonutils-go's production type auto-boxes its payload;
  fixtures compare the schema-level `query:bytes` field.

## Invariants

1. For every offline case, all listed references and tonutils-rs must
   produce `raw_hex` byte-for-byte from the same `fields`.
2. `raw_hex` must roundtrip through tonutils-rs decoders byte-for-byte
   (encode(decode(x)) == x), for offline and live cases alike.
3. Live cases (no `fields`) assert invariant 2 only; reference SDKs cannot
   reconstruct their inputs.
4. The comparison must stay reproducible in CI: both generators run in
   compare mode on every run; `--write` regenerates bytes only after
   intentional fixture changes.
5. Independently of fixtures, hard-coded constructor ids and
   schema-shaped doc blocks must match the pinned upstream `ton_api.tl`,
   with the CRC normalization calibrated against live-wire ids
   (`schema_audit.rs`).

## Crate And File Mapping

| Path | Role |
| --- | --- |
| `fixtures/cross_sdk/*.json`, `manifest.json` | canonical inputs, reference bytes, provenance metadata |
| `scripts/cross_sdk/gen_go` | tonutils-go generator: `go run .` verifies, `--write` regenerates |
| `scripts/cross_sdk/gen_py.py` | pytoniq-core generator: default verifies, `--write` regenerates |
| `crates/tonutils-tl/src/tl/cross_sdk.rs` | Rust byte-comparison and live roundtrip tests |
| `crates/tonutils-tl/src/tl/schema_audit.rs` | pinned-schema id/doc audit with live-wire CRC calibration |
| `crates/tonutils-mempool/tests/live.rs` | ignored mainnet DHT capture test |
| `.github/workflows/live-tests.yml` | `cross-sdk` CI job (Go + Python + Rust legs) |

## Verification Workflow

Run locally before committing fixture or codec changes:

```bash
(cd scripts/cross_sdk/gen_go && go run .)   # tonutils-go compare
python3 scripts/cross_sdk/gen_py.py         # pytoniq-core compare
cargo test -p tonutils-tl --lib             # tonutils-rs compare + audit
```

After intentional fixture input changes, regenerate reference bytes with
`go run . --write` in `scripts/cross_sdk/gen_go`, then re-verify with both
references.

## Live Capture

`fixtures/cross_sdk/live_dht_answers.json` holds raw mainnet bytes: the
boxed `dht.nodes` answer payload of a real `dht.findNode` exchange over
ADNL/UDP, captured before any parsing. Re-capture locally with:

```bash
TON_GLOBAL_CONFIG_JSON="$(curl -fsSL https://ton.org/global.config.json)" \
TON_CAPTURE_CROSS_SDK=1 \
cargo test -p tonutils-mempool --test live \
  capture_cross_sdk_dht_answer_from_mainnet -- --ignored
```

After re-capture, update the fixture's `sha256` and date in
`fixtures/cross_sdk/manifest.json`. The test is env-gated so plain
`--ignored` runs never overwrite committed evidence, and CI never requires
network access for these fixtures.

## Missing Work

- Live QUIC frame capture is blocked by local NAT reachability; QUIC
  framing is covered structurally by the schema audit and byte-wise by the
  tonutils-go fixtures.
- Live `adnl.packetContents` and overlay FEC broadcast captures are not
  recorded yet.
- A newer pytoniq-core schema that ships `quic.*` and `overlay.ping` would
  let those cases gain a second reference.
- Additional reference SDKs (for example tongo) are not wired in yet.
