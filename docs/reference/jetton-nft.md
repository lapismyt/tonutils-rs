# Jetton And NFT Contract Wrappers

This note records the storage layouts, embedded-code provenance, address
derivation, and get-method shapes used by `crate::jetton` and `crate::nft`.
Wrappers are edition-specific because the reference contracts differ in storage
and get-methods even when they share TEP-74 or TEP-62 behavior.

## Embedded Code

`contracts::ContractVersion` embeds one BoC hex string per supported edition.
The hex payloads are the canonical published artifacts mirrored by
`nessshon/ton-core`'s `CONTRACT_CODES`, originally built from
`ton-blockchain/token-contract` and the NFT reference contracts. Each string is
decoded lazily into an `Arc<Cell>` through a per-version `OnceLock`; a CRC32C
trailer is stripped before the local generic-BoC decoder runs.

Supported versions:

- `JettonMasterStandard`, `JettonMasterStablecoin`, `JettonMasterStablecoinV2`
- `JettonWalletStandard`, `JettonWalletStablecoin`, `JettonWalletStablecoinV2`
- `NFTCollectionStandard`, `NFTCollectionEditable`
- `NFTItemStandard`, `NFTItemEditable`, `NFTItemSoulbound`

Wrappers expose `ContractDefinition::default_code()` and constructors that take
an explicit code cell, so applications can pin their own audited build.

## Jetton Storage

All coin fields are `VarUInteger 16` and addresses are `MsgAddress`.

| Edition | Storage |
| --- | --- |
| `JettonMasterStandard` | `total_supply, admin, ^content(prefixed), ^wallet_code` |
| `JettonMasterStablecoin` | `total_supply, admin, next_admin, ^wallet_code, ^content(bare snake)` |
| `JettonWalletStandard` | `balance, owner, master, ^wallet_code` |
| `JettonWalletStablecoin` | `status:4, balance, owner, master` |
| `JettonWalletStablecoinV2` | `balance, owner, master` |

Standard masters store TEP-64 content with the `0x01` prefix; stablecoin masters
store a bare snake string. Wrappers represent both as `metadata::Tep64Content`
and choose the prefixed or bare form at serialization time.

`JettonWalletStablecoinV2` is sharded: its address is the `StateInit` hash with
the top eight bits replaced by the top eight bits of the owner hash
(`StateInit.fixed_prefix_length = Some(8)`). The result remains a standard
`Address`; no variable-length address is involved.

## NFT Storage

| Edition | Storage |
| --- | --- |
| Collection (both) | `owner, next_item_index:u64, ^content_root, ^item_code, ^royalty` |
| `NFTItemStandard` | `index:u64, collection, owner, ^content(bare)` |
| `NFTItemEditable` | `index:u64, collection, owner, ^content(bare), editor` |
| `NFTItemSoulbound` | `index:u64, collection, owner, ^content(bare), authority, revoked_at:u64` |

The collection content root holds `^collection_content(prefixed)` followed by
`^common_content(bare snake)`. Royalty parameters are
`numerator:u16, denominator:u16, address`.

An item address depends only on `index:u64` and `collection_address`, plus the
published item code. It does not include owner or content, so it can be derived
before minting. `calculate_item_address` and the collection wrappers use the
same derivation.

Soulbound items add `authority` and `revoked_at`; `revoked_at` is zero while the
token is valid and becomes a unix timestamp after revocation.

## Message Opcodes

| Operation | Opcode |
| --- | --- |
| Jetton transfer | `0xf8a7ea5` |
| Jetton internal transfer | `0x178d4519` |
| Jetton burn | `0x595f07bc` |
| Stablecoin mint | `0x642b7d07` |
| Standard mint | `0x00000015` |
| Stablecoin change admin | `0x6501f354` |
| Standard change admin | `0x00000003` |
| Stablecoin change content | `0xcb862902` |
| Standard change content | `0x00000004` |
| Jetton top-up | `0xd372158c` |
| Jetton upgrade | `0x2508d66a` |
| NFT transfer | `0x5fcc3d14` |
| NFT mint | `0x00000001` |
| NFT batch mint | `0x00000002` |
| NFT change owner | `0x00000003` |
| NFT change content | `0x00000004` |
| NFT edit content | `0x1a0b9d51` |
| NFT transfer editorship | `0x1c04412a` |
| Soulbound destroy | `0x1f04537a` |
| Soulbound revoke | `0x6f89f5e3` |

`ForwardPayload` maps to TL-B `Either 0 ^Cell`: `None` stores bit `0`, comments
store `^` cell `op=0` plus UTF-8 bytes, and arbitrary payloads store `^` cell
verbatim.

## Get-Method Shapes

| Method | Contract | Result |
| --- | --- | --- |
| `get_jetton_data` | master | `total_supply, mintable, admin, content, wallet_code` |
| `get_wallet_address` | master | `wallet_address` |
| `get_next_admin_address` | stablecoin master | `next_admin` |
| `get_wallet_data` | wallet | `balance, owner, master` and optionally `wallet_code` |
| `get_status` | stablecoin wallet | `status` |
| `get_collection_data` | collection | `next_item_index, content_root, owner` |
| `get_nft_address_by_index` | collection | `item_address` |
| `get_nft_content` | collection | `full_content` for `(index, individual_content)` |
| `royalty_params` | collection | `numerator, denominator, address` |
| `get_nft_data` | item | `init, index, collection, owner, content` |
| `get_editor` | editable item | `editor` |
| `get_authority_address` | soulbound item | `authority` |
| `get_revoked_time` | soulbound item | `revoked_at` |

Typed decoders reject extra stack entries instead of silently ignoring them.

## Wallet Deployments

`wallet::WalletOutgoingMessage` carries an optional `StateInit` alongside the
usual send mode, destination, value, body, and bounce flag. Build it from a
`contracts::DeployRequest`, either with a known destination or by deriving the
destination from the `StateInit`, and pass it to
`build_external_message_boc_outgoing` on `WalletV4R2` or `WalletV5R1`. This
deploys a contract with the same signed wallet message that pays for it.

`DeployRequest::to_internal_message` builds the message when a caller deploys
directly without a wallet.
