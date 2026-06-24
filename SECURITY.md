# Security Policy

## Reporting a vulnerability

Please report security vulnerabilities **privately** — do not open a public issue.

- Preferred: open a private security advisory via GitHub
  ([Security → Report a vulnerability](https://github.com/kerryjanes/WeftNetwork/security/advisories/new)).
- Or email **security@weftnetwork.net**.

Include a description, affected component (program, crate, or service), and reproduction steps.
We aim to acknowledge reports within 72 hours and to coordinate a fix and disclosure timeline
with you.

## Scope

The on-chain programs (`programs/`), the data-plane crates (`crates/`), and the node daemon are
the most security-sensitive areas. The protocol is designed to keep no traffic logs, and an
intermediate node cannot see traffic content or the final destination — please flag anything that
weakens these properties.

## Supported versions

Security fixes target the `main` branch and the latest release.
