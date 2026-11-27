# Security policy

CKB Service Runner will execute code published on-chain next to a full node.
Security reports are very welcome.

## Reporting a vulnerability

**Do not open a public issue.** Use GitHub's private reporting:
**Security → Report a vulnerability** on this repository.

Please include what is affected, steps to reproduce and the impact you expect.
We aim to acknowledge reports within 7 days.

## Scope

- sandbox escapes or host-call misuse
- access to node data, keys or files from a service
- leakage or non-deletion of service storage marked secret
- bypass of the full-node / sync checks
- forged or manipulated ranking data

## Status

Still pre-alpha. Mainnet runs as a node dashboard only, and the Service Runtime stays off on mainnet until the audit
findings are closed. Do not run on mainnet with value at stake. What CSR reads, sends and which privileges it has:
[docs/security-model.md](docs/security-model.md).
