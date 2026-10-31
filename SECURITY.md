# Security Policy

Bonk writes to entire disks: a flaw can destroy data. This page explains how to report a vulnerability, how to encrypt the report and what to expect in return.

French version: [SECURITY.fr.md](SECURITY.fr.md). Machine-readable policy ([RFC 9116](https://www.rfc-editor.org/rfc/rfc9116), OpenPGP-signed): <https://bonk-os.app/.well-known/security.txt>.

## Supported versions

Security fixes go to the latest published release only.

## Reporting a vulnerability

Do not open a public issue or discussion for a vulnerability. Use one of these private channels:

1. **GitHub private vulnerability reporting**: [Report a vulnerability](https://github.com/FantasmaGlad/Bonk/security/advisories/new). Only the maintainer can see the report.
2. **E-mail**: [security@bonk-os.app](mailto:security@bonk-os.app).

Please include the Bonk version, the operating system, the steps to reproduce and the effect you observe. Do not attach a personal disk image or data that identifies your hardware.

### Encrypt your report

Any e-mail containing exploitable details must be encrypted with the OpenPGP key below. If you cannot use PGP, use the private GitHub report rather than plain e-mail.

| | |
|---|---|
| Identity | Bonk Security `<security@bonk-os.app>` |
| Algorithms | Ed25519 (certify, sign) and Curve25519 (encrypt) |
| Fingerprint | `A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30` |
| Created / expires | 2026-10-31 / 2028-10-30 |
| Private key | protected by a passphrase, held by the maintainer, with a revocation certificate |

Retrieve the key from both channels and check that the fingerprint matches before use:

- this repository: [`docs/security/bonk-security-public-key.asc`](docs/security/bonk-security-public-key.asc)
- the website: <https://bonk-os.app/.well-known/security.asc>

```bash
curl -fsSL https://bonk-os.app/.well-known/security.asc | gpg --import
gpg --fingerprint security@bonk-os.app
# expected: A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30
```

To receive an encrypted reply, attach your own public key to your message.

## What to expect

| Step | Target |
|---|---|
| Acknowledgment | within 7 days |
| First assessment | within 30 days |
| Fix | as soon as practicable, announced in the release notes |
| Coordinated disclosure | with the fix, at most 90 days after the report unless agreed otherwise |

These are objectives, not contractual commitments: the project has a single maintainer. Reporters who wish it are credited.

## Scope

In scope: the code of this repository, its published binaries and installers, its workflows, the website `bonk-os.app` and the package repositories `packages.bonk-os.app`.

Out of scope: flaws in third-party services (GitHub, Cloudflare, Resend), social engineering, denial of service and mass automated scans, findings without a demonstrable impact. A vulnerability in a dependency should first be reported to its project; please tell us if it affects Bonk.

## Good-faith research

Research carried out in good faith within this scope will not lead to any action from us. Test on your own disks and images; do not access other people's data or disrupt the services.

## Verifying what we publish

Each release carries a `SHA256SUMS` file and build attestations:

```bash
gh attestation verify <file> --repo FantasmaGlad/Bonk
```

The package repositories are signed with a dedicated key, distinct from the report key above and never used to receive reports:

```bash
curl -fsSL https://packages.bonk-os.app/bonk.asc | gpg --show-keys --fingerprint
# expected: 8DE6 80FF BE47 5B66 3E8B  019A 7578 66E3 4D67 014B
```

## Document history

| Date | Change |
|---|---|
| 2026-10-31 | Report key `A782 AC57 321A 862C 2ED8 F0FB 34F1 5332 E2A0 7A30` created and published. `security.txt` signed with this key. |

This history lists every change of key, fingerprint or commitment.
