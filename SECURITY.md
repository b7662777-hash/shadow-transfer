# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x | Yes |
| < 0.1 | No |

## Reporting a vulnerability

Please do **not** open a public issue for security vulnerabilities.

Use GitHub's private vulnerability reporting for this repository
(**Security → Report a vulnerability**). Include:

- affected version/platform;
- a description of the issue and its impact;
- reproduction steps or a proof of concept;
- any relevant logs with pairing codes, file contents, personal IP addresses
  and private keys removed.

We will acknowledge the report, investigate, and coordinate disclosure. Please
allow reasonable time for a fix before publishing details.

## Scope

In scope:

- TLS/pinning/pairing weaknesses;
- path traversal or unsafe file finalization;
- integrity bypass (a file marked verified when it was not);
- denial of service caused by unbounded message/file handling;
- credential/identity key handling flaws.

Out of scope:

- issues requiring a compromised OS or local filesystem/debug access;
- LAN flooding or generic Wi-Fi attacks;
- Android OEM-specific behavior without a reproducible device/version;
- the known limitations documented in [docs/SECURITY.md](docs/SECURITY.md)
  (non-Keystore identity storage on Android, non-DPAPI storage on Windows,
  unsigned release binaries).

The full threat model and design are documented in
[docs/SECURITY.md](docs/SECURITY.md).
