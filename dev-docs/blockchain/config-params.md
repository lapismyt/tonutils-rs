# Config Parameters

TON stores network configuration on-chain. LiteAPI exposes config cells through `getConfigAll` and `getConfigParams`.

## LiteAPI

```tl
liteServer.getConfigAll mode:# id:tonNode.blockIdExt = liteServer.ConfigInfo;
liteServer.getConfigParams mode:# id:tonNode.blockIdExt param_list:(vector int) = liteServer.ConfigInfo;
```

The response contains proof bytes and config proof bytes. The config itself is encoded in TVM cells.

## Common Parameters

Commonly referenced config params include:

- `0`: config smart contract address,
- `1`: elector smart contract address,
- `2`: minter smart contract address,
- `15`: validator election timing,
- `17`: validator stake limits,
- `18`: storage prices,
- `20`: masterchain gas prices,
- `21`: basechain gas prices,
- `24`: masterchain message prices,
- `25`: basechain message prices,
- `32`: previous validator set,
- `34`: current validator set,
- `36`: next validator set.

`tlb::ConfigParams` decodes the top-level `config:^(Hashmap 32 ^Cell)` into
raw parameter cells and exposes raw-preserving wrappers for the ids above.
Exact deep TL-B schemas for each parameter still require upstream source and
fixture evidence before replacing raw cells with semantic fields.

## SDK Requirements

- Fetch config params by id.
- Decode config dictionary.
- Preserve common params behind typed ids while exact deep schemas are pending.
- Verify config proof against masterchain state.
- Keep unknown params as raw cells.

## Missing Work

- Deep typed config param models.
- Validator set decoder.
- Config proof verifier.
