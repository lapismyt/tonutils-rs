# Jettons, NFTs, and Metadata

Audience: applications that inspect or construct TEP-62 NFT and TEP-74 jetton
payloads. Prerequisites: familiarity with `tonutils-tvm` and a live provider
only for get-method helpers. Cell encoding and metadata parsing are offline.

## Choose the crate

- `tonutils-jetton` contains jetton data, transfer payloads, and optional
  provider extensions.
- `tonutils-nft` contains collection/item data and optional provider
  extensions.
- `tonutils-metadata` parses TEP-64 content and preserves raw cells for fields
  the parser does not recognize.
- `tonutils-contracts` and `tonutils-liteclient` provide the live read path.

The provider features are opt-in:

```toml
tonutils-jetton = { version = "2", features = ["provider"] }
tonutils-nft = { version = "2", features = ["provider"] }
```

## Offline boundary

Build payloads and decode known cells without credentials or a network. Keep
the resulting cell as a [`tonutils_tvm::Cell`] until a wallet or message layer
chooses how to send it. Metadata URIs are data, not verified content; callers
must apply their own fetching and trust policy.

## Live reads

Provider extensions run standard methods such as `get_jetton_data`,
`get_collection_data`, and `get_nft_data` at a selected block. A successful
response means the provider returned a decodable stack; it does not by itself
establish proof verification or asset authenticity.

Continue with [Contracts and get-methods](contracts.md), then [Wallets and
offline messages](wallets.md) when the payload is ready to submit.
