# Smart Contract Get-Methods

Get-methods are read-only TVM executions against account state.

## LiteAPI Function

```tl
liteServer.runSmcMethod mode:# id:tonNode.blockIdExt account:liteServer.accountId method_id:long params:bytes = liteServer.RunMethodResult;
```

## Method Id From Name

Common tooling maps names to ids:

```text
(crc16(method_name) & 0xffff) | 0x10000
```

## Input Stack

`params` must encode TVM stack values in the format expected by liteserver. This must be verified with real fixtures.

## Result

Important result fields:

- `exit_code`,
- optional `result`,
- optional proof fields,
- `shardblk` execution context.

Non-zero `exit_code` is a contract result, not a transport failure.

## High-Level API Design

The `contracts` module provides `Contract<'a, P>` over any `ContractProvider`.
`ContractProvider` is implemented for both `LiteClient` and `LiteBalancer`.
The wrapper provides:

- account address,
- latest-block account-state fetch,
- get-method execution by numeric method id,
- get-method execution by method name,
- typed result helpers through `RunMethodResultExt`,
- optional proof verification mode.

Proof verification mode is not implemented yet. The wrapper preserves LiteAPI
proof bytes in the response structures.

## Missing Work

- Verify stack serialization.
- Expand result stack decoding against live liteserver fixtures.
- Add known contract fixtures.
- Add proof verification.
