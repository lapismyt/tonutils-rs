# TON Wallets in Rust: Addresses, Signing, and Offline Messages

`tonutils-wallet` provides offline Wallet V4R2 and V5R1 mnemonic, state-init,
signing, and transfer-payload APIs. Enable its `provider` feature only when
binding wallet helpers to `tonutils-contracts` providers.

Audience: applications preparing wallet addresses or external messages.
Prerequisites: a supported wallet version and a secure source for mnemonic
material; only provider-backed sequence-number reads need live network access.
Goal: construct and review a message offline before choosing how to submit it.
