# TODO

This file follows the `todo-md/todo-md` format and is the detailed tracker for
work that should not be hidden in prose. Active work is currently prioritized
around idiomatic high-level LiteClient, LiteBalancer, contract, wallet,
and transport APIs. `docs/reference/` records protocol facts and source
evidence.

**NEW RELEASE SHALL NOT BE PUBLISHED UNTIL EVERYTHING DOWN BELOW IS DONE**

## `tonutils::tl`
- [x] Migrate TL schemes specific for ADNL, LiteAPI, Overlay, etc. to their respectful modules. Module `tonutils::tl` should remain only for TL types and protocol implementation. Make sure there is no duplicate names anywhere in the library afterwards.

## `tonutils::contracts`
- [ ] Just fix it. Make it work. 

## `tonutils::mempool`
- [x] Looks like some part of Overlay-communication implementation is there. Move EVERYTHING Overlay-related to the `tonutils::overlay`. `tonutils::overlay` must contain everyting to build Overlay clients and servers. And `tonutils::mempool` must be just implementation of client+server that can send and receive external messages from public Overlay.

## `tonutils::overlay`
- [x] Use Plumtree as the public-overlay broadcast path; keep signed simple/FEC broadcasting only for external-message compatibility.
- [x] Gate PRs on real DHT, UDP, QUIC, and public Plumtree receive evidence through exact single-test invocations; zero-test matches and network-unavailable skips are failures.
- [ ] Add upstream Plumtree statistics exchange and private-overlay authorized-key sets after their wire behavior is verified.

## `tonutils::provider`
- [ ] Make high-level provider abstractions that will open possibilities to connect to custom providers, i.e. Toncenter or TonAPI. Providers must be defined not by one big trait, but with separate trait for each feature, because some providers can miss some functionality and some providers can have extra functionality - i want to be able to check it on type-system level, not with runtime panics. It must replace ContractProvider.

## `tonutils::jetton`
- [ ] Reimplement from scratch.

## `tonutils::nft`
- [ ] Reimplement from scratch.

## `tonutils::emulator`
- [ ] Just create it. Make it exist.

## Cargo
- [x] Remove unused workspace dependencies. Make dependencies tightly tied to features in `tonutils`.
- [x] Remove `ureq` from `tonutils-cli` dependencies - use only `reqwest`.
- [x] Remove `anyhow` from `tonutils` dependencies - move entirely to `thiserror`.

## General
- [x] Check for bad practices: `use`s inside of functions, multiple `use` statements that can be combined, using objects via `crate::...`/`super::...` instead of importing, glob imports, senseless `allow`s, useless unit-tests. Fix it.
- [x] Sometimes when there is a need to import something, it can be tricky because of similar names in different modules of public API. Examples: `Error`, `PublicKey`, `KeyPair`, `Result`, `Int256`. I need to solve this problem completely.
- [x] Check for blocking I/O in async code, for `std` imports where should be imported from `tokio`.

## Docs
- [x] Refactor it. Make it more fresh - replace deprecated paths, names, etc. Make it more helpful for those who came from other blockchains (Solana, EVM).
