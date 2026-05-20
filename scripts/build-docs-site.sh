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

markdown_title() {
    local rel_path="$1"
    local title

    title="$(sed -n 's/^# \{1,\}//p' "$repo_root/$rel_path" | head -n 1)"
    if [[ -n "$title" ]]; then
        printf '%s\n' "$title"
        return
    fi

    basename "$rel_path" .md | tr '-' ' '
}

append_section() {
    local section_title="$1"
    shift

    if (($# == 0)); then
        return
    fi

    {
        printf '\n# %s\n\n' "$section_title"
        local rel_path
        for rel_path in "$@"; do
            printf -- '- [%s](%s)\n' "$(markdown_title "$rel_path")" "$rel_path"
        done
    } >> "$book_src/SUMMARY.md"
}

collect_markdown() {
    local dir="$1"

    if [[ ! -d "$repo_root/$dir" ]]; then
        return
    fi

    find "$repo_root/$dir" -type f -name '*.md' -printf '%p\n' \
        | sed "s|^$repo_root/||" \
        | sort
}

copy_markdown "README.md"

mapfile -t public_docs < <(collect_markdown "docs")
for rel_path in "${public_docs[@]}"; do
    copy_markdown "$rel_path"
done

mapfile -t dev_docs < <(collect_markdown "dev-docs")
for rel_path in "${dev_docs[@]}"; do
    copy_markdown "$rel_path"
done

policy_docs=()
for rel_path in CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md; do
    if [[ -f "$repo_root/$rel_path" ]]; then
        policy_docs+=("$rel_path")
        copy_markdown "$rel_path"
    fi
done

cat > "$site_root/book.toml" <<'BOOK'
[book]
title = "tonutils-rs"
language = "en"
src = "src"

[output.html]
default-theme = "light"
preferred-dark-theme = "navy"
BOOK

cat > "$book_src/SUMMARY.md" <<'SUMMARY'
# Summary

# Overview

- [tonutils-rs](README.md)
SUMMARY

append_section "Public Guides" "${public_docs[@]}"
append_section "Development Docs" "${dev_docs[@]}"
append_section "Project Policies" "${policy_docs[@]}"

mdbook build "$site_root" --dest-dir "$html_root"
