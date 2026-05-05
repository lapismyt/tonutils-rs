# TL And LiteAPI

The `tl` feature exposes Type Language structures used by ADNL and LiteAPI.
The crate keeps local schema files under `src/tl/schemas/` and maps the
implemented LiteAPI constructors into Rust enums and structs backed by
`tl-proto` serialization.

## Schema Source

LiteAPI wire types are maintained from upstream TON schemas, primarily
`src/tl/schemas/lite_api.tl`. Request constructors live in
`tonutils::tl::request`; response constructors live in
`tonutils::tl::response`; common identifiers such as `BlockIdExt`, `Int256`,
and `AccountId` live in `tonutils::tl::common`.

The repository has a schema-check test that parses the local LiteAPI schema and
compares computed constructor ids with the handwritten Rust ids. It is a drift
check for implemented constructors, not a full code generator.

## Constructor Ids

TL constructor ids are 32-bit little-endian values on the wire. Public Rust
types use `#[tl(id = "...")]` attributes, and tests verify those ids against
the local schema text where coverage exists.

When adding a constructor:

- copy the upstream TL line into the local schema file;
- add or update the Rust type with the exact constructor id;
- include vectors, flags, optional fields, and boxed enums in roundtrip tests;
- document any missing or intentionally unsupported fields in `TODO.md`.

## Typed Requests

`LiteClient::query_typed` accepts a `tonutils::tl::request::Request` value and
decodes the response into a type that implements the crate's response mapping.
The higher-level LiteClient helpers build these requests internally.

```rust
use tonutils::liteclient::client::LiteClient;
use tonutils::tl::request::Request;
use tonutils::tl::response::CurrentTime;

async fn example(client: &mut LiteClient) -> anyhow::Result<()> {
    let time: CurrentTime = client.query_typed(Request::GetTime).await?;
    println!("{}", time.now);
    Ok(())
}
```

## Raw Bytes

Use `query_raw` when the request is already serialized LiteAPI bytes or when a
constructor is known by schema but not yet modeled by the typed API.

```rust
use tonutils::liteclient::client::LiteClient;

async fn example(client: &mut LiteClient, bytes: Vec<u8>) -> anyhow::Result<()> {
    let response = client.query_raw(bytes).await?;
    println!("{}", hex::encode(response));
    Ok(())
}
```

`query_raw` preserves unknown request and response bytes. It still wraps the
payload in the ADNL LiteAPI query envelope before transport.

## Current Limits

The schema checker is active, but the local LiteAPI schema and handwritten Rust
surface are not complete. Nonfinal candidate calls, some queue and proof
helpers, and full golden binary fixtures are tracked as Phase 1 work. DHT,
overlay, and mempool TL types are future protocol work.
