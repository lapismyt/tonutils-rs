# Architecture Overview

`tonutils` is a native Rust TON SDK. Its core design constraint is autonomy: implement TON-specific protocols in this repository instead of delegating to another Rust TON SDK.

## Goals

- Pure Rust TON-specific implementation.
- No runtime `.so` dependency.
- High-performance serialization, hashing, and networking.
- Feature-gated optional modules.
- Low-level protocol access and ergonomic high-level APIs.
- Clear separation between transport errors, LiteAPI errors, TVM decoding errors, and proof verification failures.

## Layer Stack

From bottom to top:

1. Hashes, CRC, crypto helpers.
2. TL primitive and boxed serialization.
3. ADNL transport and ADNL message types.
4. LiteAPI requests and responses.
5. LiteClient request execution.
6. LiteBalancer peer selection and failover.
7. TVM cells, BoC, TLB, dictionaries, stack values.
8. Smart-contract get-method and message APIs.
9. Wallet, jetton, NFT, DHT, overlay, and mempool utilities.

## Dependency Direction

Lower layers must not depend on higher layers.

Allowed directions:

- `liteclient` may depend on `adnl`, `tl`, and `tvm`.
- `contracts` may depend on `liteclient` and `tvm`.
- `network-config` may depend on serde JSON support.
- `cli` may depend on everything needed for user commands.

Forbidden directions:

- `tvm` must not depend on `liteclient`.
- `tl` must not depend on `liteclient` except current temporary response conversion helpers; this should be removed.
- ADNL primitives must not depend on LiteAPI semantics.

## Current Implementation Anchors

- `src/adnl/`: ADNL crypto, handshake, codec, peer wrapper.
- `src/tl/`: TL request, response, common, ADNL message types.
- `src/liteclient/`: client, peer, balancer, service layers.
- `src/tvm/`: cells, BoC, addresses, dictionaries, stack.
- `src/network_config/`: global config parser.
- `src/cli/`: optional CLI.

## Public API Principles

- Expose typed methods for stable, common workflows.
- Expose raw byte escape hatches for schema-forward compatibility.
- Preserve proof bytes even when full verification is not implemented.
- Do not silently trust data just because it came from a liteserver.
- Use explicit mode/flag builders where boolean argument lists become ambiguous.

## Missing Architecture Work

- Split TL response conversion out of `tl::utils` to remove the `tl -> liteclient` dependency.
- Add a shared request-executor trait so `LiteClient` and `LiteBalancer` do not duplicate every method.
- Add `contracts` module as a high-level API layer.
- Define stable error enums per subsystem.
