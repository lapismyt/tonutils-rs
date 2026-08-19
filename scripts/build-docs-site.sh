#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
site_root="$repo_root/target/mdbook-site-src"
book_src="$site_root/src"
html_root="$repo_root/target/mdbook-site-html"

rm -rf "$site_root" "$html_root"
mkdir -p "$book_src" "$html_root"

copy_markdown() {
    local rel_path="$1"
    mkdir -p "$book_src/$(dirname "$rel_path")"
    cp "$repo_root/$rel_path" "$book_src/$rel_path"
}

for rel_path in README.md \
    docs/getting-started.md docs/examples.md docs/tvm.md docs/tl.md \
    docs/networking.md docs/liteclient.md docs/balancer.md docs/contracts.md \
    docs/wallets.md docs/jettons-nft.md docs/cli.md docs/testing.md; do
    copy_markdown "$rel_path"
done

# Keep implementation notes in the same book, but make their purpose explicit.
while IFS= read -r rel_path; do copy_markdown "$rel_path"; done < <(
    find "$repo_root/dev-docs" -type f -name '*.md' -printf '%p\n' \
        | sed "s|^$repo_root/||" | sort
)

for rel_path in CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md; do
    if [[ -f "$repo_root/$rel_path" ]]; then copy_markdown "$rel_path"; fi
done

cat > "$site_root/book.toml" <<'BOOK'
[book]
title = "tonutils-rs documentation"
authors = ["tonutils-rs contributors"]
description = "A task-oriented guide to the pure-Rust TON SDK, its protocol primitives, and operational notes."
language = "en"
src = "src"

[build]
build-dir = "../mdbook-site-html"

[output.html]
default-theme = "light"
preferred-dark-theme = "navy"
smart-punctuation = true

[output.html.playground]
runnable = false

[output.html.search]
enable = true
limit-results = 30
BOOK

cat > "$book_src/SUMMARY.md" <<'SUMMARY'
# Summary

# Learn the library

- [Project overview](README.md)
- [Getting started](docs/getting-started.md)
- [First offline example](docs/examples.md)
- [TVM, cells, and BoC](docs/tvm.md)
- [TL and TL-B](docs/tl.md)
- [Networking and configuration](docs/networking.md)
- [LiteClient](docs/liteclient.md)
- [LiteBalancer and rate limiting](docs/balancer.md)
- [Contracts and get-methods](docs/contracts.md)
- [Wallets and offline messages](docs/wallets.md)
- [Jettons, NFTs, and metadata](docs/jettons-nft.md)
- [CLI and schema tools](docs/cli.md)
- [Testing and live-network workflows](docs/testing.md)

# Internal reference

## Architecture and feature model

- [Architecture](dev-docs/architecture/overview.md)
- [Features](dev-docs/architecture/features.md)
- [Error model](dev-docs/architecture/errors.md)
- [Public API inventory](dev-docs/api/public-api.md)
- [Examples policy](dev-docs/api/examples.md)

## TVM, TL-B, and BoC

- [Addresses](dev-docs/tvm/addresses.md)
- [Cells](dev-docs/tvm/cells.md)
- [BoC](dev-docs/tvm/boc.md)
- [Dictionaries](dev-docs/tvm/dictionaries.md)
- [Stack](dev-docs/tvm/stack.md)
- [TL-B](dev-docs/tvm/tlb.md)
- [Schema language](dev-docs/tl/schema-language.md)

## LiteAPI, ADNL, and network

- [LiteAPI](dev-docs/tl/lite-api.md)
- [Request flow](dev-docs/liteclient/request-flow.md)
- [Proofs](dev-docs/liteclient/proofs.md)
- [Workflow coverage](dev-docs/liteclient/workflow-coverage.md)
- [Balancer](dev-docs/liteclient/balancer.md)
- [Rate limiting](dev-docs/liteclient/rate-limiting.md)
- [ADNL over TCP](dev-docs/network/adnl-tcp.md)
- [ADNL over UDP](dev-docs/network/adnl-udp.md)
- [DHT](dev-docs/network/dht.md)
- [Overlays](dev-docs/network/overlays.md)
- [Global config](dev-docs/network/global-config.md)

## Blockchain and contracts

- [Blockchain data model](dev-docs/blockchain/data-model.md)
- [Messages](dev-docs/blockchain/messages.md)
- [Sharding](dev-docs/blockchain/sharding.md)
- [Config parameters](dev-docs/blockchain/config-params.md)
- [Block config proofs](dev-docs/blockchain/block-config-proof.md)
- [Contract messages](dev-docs/contracts/messages.md)
- [Get-methods](dev-docs/contracts/get-methods.md)
- [Wallet V4R2 mnemonics](dev-docs/contracts/wallet-v4r2-mnemonics.md)
- [Wallet V5R1](dev-docs/contracts/wallet-v5r1.md)
- [TEP metadata](dev-docs/contracts/tep-metadata.md)

## Testing, fixtures, and operations

- [Testing strategy](dev-docs/testing/strategy.md)
- [Fixtures](dev-docs/testing/fixtures.md)
- [Benchmarks](dev-docs/testing/benchmarks.md)
- [Diagnostics](dev-docs/operations/diagnostics.md)
- [Source tracking](dev-docs/operations/source-tracking.md)

## Schema inventory and research

- [Schema inventory](dev-docs/schema-inventory.tsv)
- [Mempool research](dev-docs/research/mempool.md)

# Project policies

- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security](SECURITY.md)
SUMMARY

mdbook build "$site_root"
