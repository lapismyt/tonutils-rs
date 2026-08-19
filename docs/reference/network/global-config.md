# Global Config And Liteserver Descriptors

TON global config JSON gives static entry points for clients. The crate currently parses liteserver entries.

## Liteserver Descriptor

Fields:

- `ip`: signed 32-bit integer containing IPv4 bits,
- `port`: TCP port,
- `id`: public key object.

Public key form:

```json
{
  "@type": "pub.ed25519",
  "key": "base64..."
}
```

## IP Conversion

The signed integer should be reinterpreted as `u32` bits, then converted to `Ipv4Addr`.

Edge cases:

- negative input,
- `0.0.0.0`,
- `127.0.0.1`,
- `255.255.255.255`.

## Feature Boundary

- JSON parsing: `network-config`.
- HTTP download: `cli` or future optional HTTP feature.

Library users should be able to provide config JSON without enabling HTTP.

## Tests

Required tests:

- base64 public key decode,
- invalid key rejection,
- signed IP roundtrip,
- socket address creation,
- serialization roundtrip.
