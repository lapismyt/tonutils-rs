# TON Addresses

## Purpose And Scope

TON account addresses identify smart-contract accounts. This page documents the
address behavior currently implemented by `src/tvm/address.rs` for the Phase 1
TVM foundation slice. It covers internal standard addresses, raw text form,
user-friendly base64 forms, validation rules, and crate mapping. External
addresses remain a small value/bit-length helper and are not yet a complete
TL-B `addr_extern` implementation.

## Data Model

The current `Address` stores:

- `workchain: i8`, with `-1` for masterchain and `0` for basechain in normal
  usage,
- `hash_part: [u8; 32]`, the 256-bit account id,
- `is_bounceable: bool`,
- `is_test_only: bool`.

Because the public type stores `i8`, the strict parser accepts `-1` and
`0..=127`. Values below `-1` are rejected as invalid workchain ids for this
type. Full TL-B `addr_std` can carry a signed 8-bit anycast-free workchain in
the serialized form and broader message-level address handling remains future
work.

## Raw Format

Raw textual form is:

```text
workchain:64_hex_chars
```

Examples:

```text
0:0000000000000000000000000000000000000000000000000000000000000000
-1:2222222222222222222222222222222222222222222222222222222222222222
```

Validation rules:

- exactly one `:` separator,
- decimal workchain parseable as `i8`,
- workchain must be `-1` or `0..=127`,
- hash must be exactly 64 hexadecimal characters,
- invalid hex is reported as an address hash error.

`to_raw()` and `to_hex()` both produce this form. `Display` intentionally
continues to produce the user-friendly URL-safe form for compatibility with the
existing API.

## User-Friendly Format

User-friendly addresses encode 36 bytes:

```text
tag:1 | workchain:1 | account_id:32 | crc16:2
```

The CRC16 is computed over the first 34 bytes and stored in big-endian byte
order. The supported tag values are:

- `0x11`: bounceable,
- `0x51`: non-bounceable,
- `tag | 0x80`: test-only variant of either supported tag.

The parser accepts all four common base64 input encodings:

- URL-safe without padding,
- URL-safe with padding,
- standard base64 without padding,
- standard base64 with padding.

Formatting helpers:

- `to_base64()` and `to_user_friendly_url_safe()` produce URL-safe base64
  without padding using the stored flags,
- `to_user_friendly_base64()` produces standard base64 with padding using the
  stored flags,
- `to_bounceable(url_safe)` and `to_non_bounceable(url_safe)` override the
  bounceability flag for output,
- `to_test_only(url_safe)` and `to_non_test_only(url_safe)` override the
  test-only flag for output,
- the legacy `to_string(user_friendly, url_safe, bounceable, test_only)` helper
  is preserved.

## Invariants And Edge Cases

The decoder rejects:

- base64 payloads that do not decode to exactly 36 bytes,
- tags other than `0x11`, `0x51`, `0x91`, or `0xd1`,
- CRC16 mismatches,
- workchain bytes that map to values below `-1`,
- malformed raw hash or workchain values.

The parser preserves parsed bounceable/non-bounceable and test-only flags in the
returned `Address`. Raw format has no flag storage, so raw parsing creates a
bounceable, non-test-only address by default.

## Fixture Coverage

Current embedded fixture tests cover the TON Docs address example:

```text
0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e
```

The test vectors include bounceable, non-bounceable, bounceable test-only, and
non-bounceable test-only forms in both URL-safe and standard base64 alphabets.
They validate tag parsing, flag preservation, CRC16 checking, and conversion
back to raw form.

The zero account id is also covered as a synthetic edge fixture for known
bounceable and non-bounceable encodings.

## LiteAPI Mapping

LiteAPI uses:

```tl
liteServer.accountId workchain:int id:int256 = liteServer.AccountId;
```

`Address::to_account_id()` maps the stored `i8` workchain into the LiteAPI
`int` field and copies the 32-byte hash into `int256`.

## Missing Work

- Full TL-B `addr_none`, `addr_extern`, `addr_std`, and `addr_var` models.
- Anycast support.
- Additional captured fixtures from upstream TON or pytoniq-core for non-zero
  masterchain addresses and address values seen in live contract workflows.
- Public address inspection/formatting CLI commands.
