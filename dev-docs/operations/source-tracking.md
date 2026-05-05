# Source Tracking

Protocol documentation and implementation must record where facts came from.

## Source Priority

1. Upstream `ton-blockchain/ton` schemas and implementation.
2. Official TON docs.
3. Captured behavior from public liteservers with fixtures.
4. Mature SDKs such as `tonutils-go`, `tongo`, `ton-rs`, and `tonlib-rs`.
5. Existing crate behavior.

## Reference Catalog

Primary sources:

- Upstream TON implementation and schemas: https://github.com/ton-blockchain/ton
- Official TON documentation index for LLM-assisted research: https://docs.ton.org/llms.txt

Compatibility references:

- `tonutils-go`: https://github.com/xssnick/tonutils-go
- `tongo`: https://github.com/tonkeeper/tongo
- STON.fi `ton-rs`: https://github.com/ston-fi/ton-rs
- STON.fi `tonlib-rs`: https://github.com/ston-fi/tonlib-rs
- `nessshon/tonutils`: https://github.com/nessshon/tonutils

Research references:

- TON mempool scanner behavior: https://github.com/yungwine/ton-mempool

These projects are references for protocol behavior, API ergonomics, and fixture comparison. They must not be treated as dependency approval, and Rust TON SDK crates must not be added as runtime dependencies.

## What To Record

For each synced schema or fixture:

- source URL or local path,
- upstream commit if known,
- date fetched,
- relevant constructor names,
- expected ids or hashes,
- compatibility notes.

## Local Schema Policy

Local schemas under `src/tl/schemas/` should have an adjacent note or generated metadata recording upstream origin. The long-term goal is automated schema sync validation.

## Fixture Policy

Fixtures should include metadata files or comments explaining:

- whether they are synthetic or captured,
- exact input bytes,
- expected decoded values,
- source implementation used for comparison.

## Missing Work

- Add `dev-docs/sources.md` or generated source metadata.
- Add schema sync command.
- Add fixture metadata format.
