# Security

Bonk writes to entire disks: a flaw can destroy data.

[Version française](SECURITY.fr.md)

## Report

Privately, never in a public issue:

- [GitHub private report](https://github.com/FantasmaGlad/Bonk/security/advisories/new)
- security@bonk-os.app, encrypted

Give the version, the system, the steps and the effect.

## Encrypt

```bash
gpg --locate-keys security@bonk-os.app
gpg --fingerprint security@bonk-os.app
```

Expected fingerprint:

`A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30`

## What to expect

- Acknowledgment within 7 days
- First assessment within 30 days
- Disclosure with the fix, within 90 days

These are goals: the project has a single maintainer.

## Scope

The latest release, the websites and the package repositories. Good-faith research is welcome.
