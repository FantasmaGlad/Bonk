# Security

Bonk writes to entire disks: a flaw can destroy data. [Version française](SECURITY.fr.md)

Report a vulnerability privately, never in a public issue: through a [GitHub private report](https://github.com/FantasmaGlad/Bonk/security/advisories/new), or by an encrypted mail to security@bonk-os.app. Give the version, the system, the steps and the effect.

Fetch the key and check its fingerprint before encrypting:

```bash
gpg --locate-keys security@bonk-os.app
gpg --fingerprint security@bonk-os.app
```

`A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30`

The goals are an acknowledgment within 7 days, a first assessment within 30 days and disclosure with the fix within 90 days. They are goals, not promises: the project has a single maintainer.

The policy covers the latest release, the websites and the package repositories. Good-faith research is welcome.
