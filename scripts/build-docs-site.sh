#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
site_root="$repo_root/target/mdbook-site-src"
book_src="$site_root/src"
html_root="$repo_root/target/mdbook-site-html"
site_url="https://lapismyt.github.io/tonutils-rs/"

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
    find "$repo_root/docs/reference" -type f -name '*.md' -printf '%p\n' \
        | sed "s|^$repo_root/||" | sort
)

copy_markdown docs/reference/schema-inventory.tsv

for rel_path in CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md; do
    if [[ -f "$repo_root/$rel_path" ]]; then copy_markdown "$rel_path"; fi
done

cat > "$site_root/book.toml" <<'BOOK'
[book]
title = "tonutils-rs | Pure-Rust TON SDK documentation"
authors = ["tonutils-rs contributors"]
description = "Task-oriented Rust guides for TON TVM, TL-B, LiteAPI, wallets, contracts, and network workflows."
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
limit-results = 50
teaser-word-count = 30
BOOK

cat > "$book_src/SUMMARY.md" <<'SUMMARY'
# Summary

# Public guides

- [Overview: pure-Rust TON SDK](README.md)
- [Getting started: crates and features](docs/getting-started.md)
- [Examples: offline and live workflows](docs/examples.md)
- [TVM: cells, BoC, and stack values](docs/tvm.md)
- [TL and LiteAPI serialization](docs/tl.md)
- [Networking: ADNL and global config](docs/networking.md)
- [LiteClient: LiteAPI queries](docs/liteclient.md)
- [LiteBalancer: multi-peer requests](docs/balancer.md)
- [Contracts: state and get-methods](docs/contracts.md)
- [Wallets: signing and offline messages](docs/wallets.md)
- [Jettons, NFTs, and metadata](docs/jettons-nft.md)
- [CLI: queries, BoC, and schemas](docs/cli.md)
- [Testing: offline and live checks](docs/testing.md)

# Internal reference

## Architecture and feature model

- [Architecture](docs/reference/architecture/overview.md)
- [Features](docs/reference/architecture/features.md)
- [Error model](docs/reference/architecture/errors.md)
- [Public API inventory](docs/reference/api/public-api.md)
- [Examples policy](docs/reference/api/examples.md)

## TVM, TL-B, and BoC

- [Addresses](docs/reference/tvm/addresses.md)
- [Cells](docs/reference/tvm/cells.md)
- [BoC](docs/reference/tvm/boc.md)
- [Dictionaries](docs/reference/tvm/dictionaries.md)
- [Stack](docs/reference/tvm/stack.md)
- [TL-B](docs/reference/tvm/tlb.md)
- [Schema language](docs/reference/tl/schema-language.md)

## LiteAPI, ADNL, and network

- [LiteAPI](docs/reference/tl/lite-api.md)
- [Request flow](docs/reference/liteclient/request-flow.md)
- [Proofs](docs/reference/liteclient/proofs.md)
- [Workflow coverage](docs/reference/liteclient/workflow-coverage.md)
- [Balancer](docs/reference/liteclient/balancer.md)
- [Rate limiting](docs/reference/liteclient/rate-limiting.md)
- [ADNL over TCP](docs/reference/network/adnl-tcp.md)
- [ADNL over UDP](docs/reference/network/adnl-udp.md)
- [DHT](docs/reference/network/dht.md)
- [Overlays](docs/reference/network/overlays.md)
- [Global config](docs/reference/network/global-config.md)

## Blockchain and contracts

- [Blockchain data model](docs/reference/blockchain/data-model.md)
- [Messages](docs/reference/blockchain/messages.md)
- [Sharding](docs/reference/blockchain/sharding.md)
- [Config parameters](docs/reference/blockchain/config-params.md)
- [Block config proofs](docs/reference/blockchain/block-config-proof.md)
- [Contract messages](docs/reference/contracts/messages.md)
- [Get-methods](docs/reference/contracts/get-methods.md)
- [Wallet V4R2 mnemonics](docs/reference/contracts/wallet-v4r2-mnemonics.md)
- [Wallet V5R1](docs/reference/contracts/wallet-v5r1.md)
- [TEP metadata](docs/reference/contracts/tep-metadata.md)

## Testing, fixtures, and operations

- [Testing strategy](docs/reference/testing/strategy.md)
- [Fixtures](docs/reference/testing/fixtures.md)
- [Benchmarks](docs/reference/testing/benchmarks.md)
- [Diagnostics](docs/reference/operations/diagnostics.md)
- [Source tracking](docs/reference/operations/source-tracking.md)

## Schema inventory and research

- [Schema inventory](docs/reference/schema-inventory.tsv)
- [Mempool research](docs/reference/research/mempool.md)

# Project policies

- [Contributing](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security](SECURITY.md)
SUMMARY

mdbook build "$site_root"

python3 - "$html_root" "$site_url" <<'PY'
from html import escape
from pathlib import Path
import re
import sys

html_root = Path(sys.argv[1])
site_url = sys.argv[2]

descriptions = {
    "index.html": "tonutils-rs is a pure-Rust TON SDK for TVM, TL-B, LiteAPI, contracts, wallets, jettons, NFTs, and CLI workflows.",
    "docs/getting-started.html": "Choose tonutils-rs crates and Cargo features for offline TON data or live LiteAPI applications.",
    "docs/examples.html": "Compile and run tonutils-rs examples for TON cells, TL-B, LiteAPI, contracts, and wallets.",
    "docs/tvm.html": "Build and decode TON TVM cells, BoC payloads, addresses, dictionaries, and stack values in Rust.",
    "docs/tl.html": "Serialize TON TL and LiteAPI constructors and connect schema checks to Rust protocol types.",
    "docs/networking.html": "Configure TON ADNL TCP, LiteAPI peers, and global network configuration in Rust.",
    "docs/liteclient.html": "Use tonutils-liteclient for typed and raw TON LiteAPI queries, contract reads, and rate limits.",
    "docs/balancer.html": "Route TON LiteAPI requests across multiple liteservers with LiteBalancer retries and limits.",
    "docs/contracts.html": "Read TON account state and run contract get-methods through provider helpers in Rust.",
    "docs/wallets.html": "Derive TON wallet addresses and build signed Wallet V4R2 and V5R1 messages offline.",
    "docs/jettons-nft.html": "Encode and decode TON jetton, NFT, and TEP-64 metadata payloads in Rust.",
    "docs/cli.html": "Use the tonutils CLI for TON queries, contract calls, BoC conversion, and schema checks.",
    "docs/testing.html": "Run deterministic tonutils-rs checks, compile examples, and isolate live-network tests.",
}

def page_url(relative: str) -> str:
    if relative == "index.html":
        return site_url
    return site_url + relative

for html_file in html_root.rglob("*.html"):
    relative = html_file.relative_to(html_root).as_posix()
    contents = html_file.read_text(encoding="utf-8")
    description = descriptions.get(
        relative,
        "Internal tonutils-rs protocol, architecture, testing, and implementation notes.",
    )
    title = "tonutils-rs documentation"
    if relative == "index.html":
        title = "tonutils-rs | Pure-Rust TON SDK"
    elif relative.startswith("docs/"):
        title = "tonutils-rs | " + relative.removeprefix("docs/").removesuffix(".html").replace("-", " ").title()
    canonical = page_url(relative)
    head = (
        f'<title>{escape(title)}</title>\n'
        f'<meta name="description" content="{escape(description)}">\n'
        f'<link rel="canonical" href="{escape(canonical)}">\n'
        f'<meta property="og:type" content="website">\n'
        f'<meta property="og:title" content="{escape(title)}">\n'
        f'<meta property="og:description" content="{escape(description)}">\n'
        f'<meta property="og:url" content="{escape(canonical)}">\n'
        f'<meta name="twitter:card" content="summary">\n'
        f'<meta name="twitter:title" content="{escape(title)}">\n'
        f'<meta name="twitter:description" content="{escape(description)}">\n'
        f'<meta name="twitter:url" content="{escape(canonical)}">'
    )
    contents = re.sub(r'<meta name="description"[^>]*>\s*', "", contents)
    contents = re.sub(r"<title>.*?</title>", f"<title>{escape(title)}</title>", contents, count=1, flags=re.DOTALL)
    contents = contents.replace("</title>", "</title>\n" + head.split("\n", 1)[1], 1)
    html_file.write_text(contents, encoding="utf-8")

public_pages = ["index.html"] + [f"docs/{name}.html" for name in (
    "getting-started", "examples", "tvm", "tl", "networking", "liteclient",
    "balancer", "contracts", "wallets", "jettons-nft", "cli", "testing",
)]
sitemap_entries = [
    f"  <url>\n    <loc>{escape(page_url(path))}</loc>\n  </url>"
    for path in public_pages
    if (html_root / path).is_file()
]
(html_root / "sitemap.xml").write_text(
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
    + "\n".join(sitemap_entries)
    + "\n</urlset>\n",
    encoding="utf-8",
)
(html_root / "robots.txt").write_text(
    "User-agent: *\nAllow: /\nSitemap: " + site_url + "sitemap.xml\n",
    encoding="utf-8",
)
PY
