# Security Policy

## Reporting A Vulnerability

Do not open public GitHub issues for vulnerabilities or suspected
vulnerabilities.

Use GitHub Private Vulnerability Reporting or a GitHub Security Advisory for
this repository so maintainers can investigate before details are public. If
private reporting is unavailable, contact the maintainers through their GitHub
profiles and include only the minimum information needed to establish a private
reporting channel.

Helpful reports include:

- Affected crate version or commit.
- Affected feature flags and operating environment.
- Impact, expected attacker capability, and affected users.
- Reproduction steps, minimized proof-of-concept details, or malformed inputs.
- Whether live-network credentials, private keys, seed phrases, or user funds
  could be exposed or affected.

Never include real private keys, seed phrases, production credentials,
non-public user data, or funds-bearing live wallet material in a report. If a
report requires sensitive evidence, first establish a private maintainer
channel and agree on a safe transfer method.

## Supported Versions

Security fixes are prioritized for the latest published release and the current
`main` branch. Older releases may receive fixes when the issue is severe and a
small, low-risk backport is practical.

## Handling Sensitive Material

Use deterministic test fixtures whenever possible. Do not attach production TON
global configs, private liteserver credentials, mnemonic phrases, private keys,
access tokens, logs containing secrets, or non-public account data.

If you accidentally disclose sensitive material in a report, say so immediately
in the private thread so maintainers can help coordinate rotation, revocation,
or repository cleanup.
