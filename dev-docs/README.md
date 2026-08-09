# TON Development Documentation

This directory is the internal technical reference for implementing `tonutils`. It is intentionally more implementation-oriented than general TON documentation: each page connects protocol facts to concrete Rust modules, invariants, tests, and missing work.

Human contributors and AI agents should read this directory before changing
protocol behavior. Public user guides belong in `docs/`; protocol evidence,
wire formats, invariants, source priorities, and crate mapping belong here.

## Reading Order

1. [Architecture overview](architecture/overview.md)
2. [Feature matrix](architecture/features.md)
3. [Source tracking](operations/source-tracking.md)
4. [Crypto primitives](crypto/primitives.md)
5. [TL schema language](tl/schema-language.md)
6. [Checked-in schema inventory](schema-inventory.tsv)
7. [LiteAPI schema](tl/lite-api.md)
8. [ADNL TCP](network/adnl-tcp.md)
9. [TVM cells](tvm/cells.md)
10. [BoC format](tvm/boc.md)
11. [TL-B data models](tvm/tlb.md)
12. [Blockchain data model](blockchain/data-model.md)
13. [Blockchain TL-B coverage](blockchain/tlb-coverage.md)
14. [Block, config, and proof TL-B slice](blockchain/block-config-proof.md)
15. [LiteClient request flow](liteclient/request-flow.md)
16. [LiteClient rate limiting](liteclient/rate-limiting.md)
17. [Smart-contract get-methods](contracts/get-methods.md)
19. [Wallet V5R1](contracts/wallet-v5r1.md)
20. [Wallet V4R2 and TON mnemonics](contracts/wallet-v4r2-mnemonics.md)
21. [TEP metadata roadmap](contracts/tep-metadata.md)

## Directory Map

- `architecture/`: crate layers, features, errors, performance policy.
- `api/`: public API design, compatibility and ergonomics.
- `blockchain/`: blocks, accounts, transactions, messages, config params.
- `crypto/`: hashes, checksums, keys, signatures, encryption primitives.
- `tl/`: TL syntax, schema maintenance, LiteAPI types and function mapping.
- `network/`: ADNL transport, DHT, overlays, global config.
- `tvm/`: cells, BoC, addresses, dictionaries, TL-B, TVM stack.
- `liteclient/`: request flow, balancer, proof verification.
- `contracts/`: get-methods, external messages, high-level contract API.
- `operations/`: source tracking, diagnostics, maintenance workflow.
- `research/`: mempool scanning notes and future protocol investigations.
- `testing/`: fixtures, live tests, benchmarks.

## Documentation Contract

Every topic file should answer:

- What TON subsystem does this describe?
- Which wire formats, constructor ids, byte order, limits, and flags matter?
- What invariants must code preserve?
- Which files in this crate implement or will implement it?
- Which tests or fixtures prove compatibility?
- What is still missing?

Repository text must stay English-only.

## Source Of Truth Priority

When sources disagree, prefer this order:

1. Upstream `ton-blockchain/ton` schemas and C++ implementation.
2. Official TON documentation and specs.
3. Behavior observed from public liteservers with recorded fixtures.
4. Mature SDK behavior such as `tonutils-go`, `tongo`, pytoniq, and
   pytoniq-core.
5. Existing crate behavior.

Use pytoniq and pytoniq-core for capability inspiration or comparison evidence
after upstream TON facts are established. They are not API or structure parity
targets. Record any deliberate protocol compatibility deviation in `TODO.md`
and in the relevant subsystem document.
