set shell := ["bash", "-euo", "pipefail", "-c"]

# List available recipes.
default:
    @just --list

# Format the workspace.
fmt:
    cargo fmt --all

# Check formatting without modifying files.
fmt-check:
    cargo fmt --all -- --check

# Lint the whole workspace with every feature and target.
check:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Lint the CLI with its declared features.
check-cli:
    cargo clippy -p tonutils-cli --all-targets -- -D warnings

# Run the workspace test suite.
test:
    cargo test --workspace --all-features

# Validate the checked-in schema metadata and inventory.
schema:
    cargo run -p tonutils-schema-gen -- check

# Check licenses, advisories, bans, and sources.
deny:
    cargo deny check

# Run every CI gate locally.
ci: fmt-check check-cli check test schema deny
