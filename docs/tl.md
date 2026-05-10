# TL And LiteAPI

The `tl` feature exposes Type Language structures used by ADNL and LiteAPI.
The crate keeps local schema files under `src/tl/schemas/` and maps the
implemented LiteAPI constructors into Rust enums and structs backed by
`tl-proto` serialization.

Audience: contributors adding LiteAPI constructors or callers using typed and
raw LiteAPI requests. Prerequisites: `tl` for schema types, `liteclient` for
network transport, and upstream TON schema evidence before changing wire types.

## Schema Source

LiteAPI wire types are maintained from upstream TON schemas, primarily
`src/tl/schemas/lite_api.tl`. Request constructors live in
`tonutils::tl::request`; response constructors live in
`tonutils::tl::response`; common identifiers such as `BlockIdExt`, `Int256`,
and `AccountId` live in `tonutils::tl::common`.

The repository has a schema-check test that parses the local LiteAPI schema and
compares computed constructor ids with the handwritten Rust ids. It is a drift
check for implemented constructors, not a full code generator.

## TL-B Schema Workflow

The `tvm` feature also exposes a small TL-B schema workflow in
`tonutils::tlb::schema`. Phase 1 keeps an upstream-derived slice of
`ton-blockchain/ton` `crypto/block/block.tlb` at
`src/tlb/schemas/block_phase1.tlb` and a checked generated summary at
`src/tlb/generated/block_phase1.rs`.

Run the deterministic check with:

```bash
tonutils tvm schema check
cargo test --lib tlb::schema
```

The current generator parses constructor tags, implicit tags, references,
grouped references, `Maybe`, `Either`, fixed-width integer/bit expressions,
`VarUInteger`, `HashmapE`, `HashmapAug`, `HashmapAugE`, and bounded constraint
text well enough to detect drift in the Phase 1 block/config/proof slice.
Message, account, and transaction types remain hand-written canonical public
models; the generated-backed slice covers block, config, shard-state, and
Merkle proof/update wrappers while deeper model generation remains tracked in
`TODO.md`.

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
surface are not complete. Nonfinal candidate calls, queue helpers, and several
proof helpers exist, but broader generated coverage and full golden binary
fixtures remain follow-up work. DHT, overlay, and mempool TL types are future
protocol work.
