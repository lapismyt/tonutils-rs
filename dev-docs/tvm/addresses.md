# TON Addresses

TON account addresses identify smart-contract accounts.

## Internal Address

An internal standard address contains:

- optional anycast,
- workchain id,
- 256-bit account id.

Common workchains:

- `-1`: masterchain,
- `0`: basechain.

## Raw Format

Raw textual form:

```text
workchain:64_hex_chars
```

Example:

```text
0:0123456789abcdef...
```

## User-Friendly Format

User-friendly addresses are base64-encoded structures with:

- tag byte,
- workchain byte,
- 32-byte account id,
- CRC16 checksum.

Flags include:

- bounceable/non-bounceable,
- testnet-only,
- URL-safe variant.

## LiteAPI Mapping

LiteAPI uses:

```tl
liteServer.accountId workchain:int id:int256 = liteServer.AccountId;
```

## Tests Needed

- raw parse and format,
- bounceable base64,
- non-bounceable base64,
- URL-safe base64,
- testnet flag,
- checksum rejection.
