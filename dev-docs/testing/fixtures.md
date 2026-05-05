# Fixture Policy

Fixtures are required for compatibility with official TON behavior.

## Fixture Metadata

Every fixture should document:

- source,
- date,
- upstream commit if known,
- schema file version,
- expected decoded structure,
- whether it is synthetic or captured.

## Fixture Types

- TL binary constructors.
- ADNL frames.
- BoC files.
- Cell hashes.
- Account states.
- Block proofs.
- Get-method results.

## Storage Rules

- Keep binary fixtures small.
- Prefer hex for very small values.
- Use files for larger BoC or network captures.
- Never include private keys or sensitive live credentials.
