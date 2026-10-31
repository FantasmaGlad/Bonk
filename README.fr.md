# Bonk

<p align="center">
  <b>Écrit des images amorçables sur clé USB. Un seul binaire, ligne de commande et fenêtre, aucune télémétrie.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Téléchargements</b></a> &nbsp;·&nbsp;
  <a href="README.md"><b>English</b></a> &nbsp;·&nbsp;
  <a href="docs/ARCHITECTURE.md"><b>Architecture</b></a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Version&color=brightgreen" alt="Version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/Licence-MIT-blue.svg" alt="Licence: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI + GUI">
  <img src="https://img.shields.io/badge/Rust-2024-orange?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/Plateforme-Linux-success?logo=linux&logoColor=white" alt="Plateforme: Linux">
  <img src="https://img.shields.io/badge/Preview-Windows%20%7C%20macOS-yellow" alt="Preview: Windows, macOS">
</p>

---

## Démarrer

```bash
tar xzf bonk-0.0.2-linux-x86_64.tar.gz
./bonk list
./bonk write <id> https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/debian-13.7.0-amd64-netinst.iso \
  --sha256-file https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/SHA256SUMS --yes
```

`bonk write` accepte une adresse ou un fichier local (`.iso`, `.img`, `.gz`, `.xz`, `.zst`), le contrôle, l'écrit et le relit. Une ISO Windows 10 ou 11 x86_64 devient une clé d'installation UEFI. Pas de `sudo` : Linux demande l'autorisation par polkit. `bonk` seul ouvre la fenêtre.

## Télécharger

| Plateforme | Format | Architecture | Téléchargement | État |
| :--- | :---: | :---: | :---: | :--- |
| <img src="https://cdn.simpleicons.org/debian" width="18" height="18" valign="middle" alt="Debian"> **Debian, Ubuntu** | `.deb` | amd64, arm64 | [amd64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk_0.0.2-1_amd64.deb) · [arm64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk_0.0.2-1_arm64.deb) | Écrit les disques |
| <img src="https://cdn.simpleicons.org/fedora" width="18" height="18" valign="middle" alt="Fedora"> **Fedora, RHEL, SUSE** | `.rpm` | x86_64, aarch64 | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.x86_64.rpm) · [aarch64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.aarch64.rpm) | Écrit les disques |
| <img src="https://cdn.simpleicons.org/linux" width="18" height="18" valign="middle" alt="Linux"> **Tout Linux** | `.tar.gz` | x86_64, aarch64, i686 | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64.tar.gz) · [aarch64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64.tar.gz) · [i686](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-i686.tar.gz) | Écrit les disques |
| <img src="https://cdn.simpleicons.org/linux" width="18" height="18" valign="middle" alt="Linux"> **Statique, ligne de commande** | `.tar.gz` | x86_64, aarch64 | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64-musl.tar.gz) · [aarch64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64-musl.tar.gz) | Écrit les disques |
| **Windows 10+** | `.zip` | x86_64, ARM64, x86 | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-x86_64.zip) · [ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-aarch64.zip) · [x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-i686.zip) | Aperçu : pas encore d'accès aux disques |
| <img src="https://cdn.simpleicons.org/apple" width="18" height="18" valign="middle" alt="macOS"> **macOS 12+** | `.tar.gz` | Universel | [Universel](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-macos-universal.tar.gz) | Aperçu : pas encore d'accès aux disques |

Empreintes : [`SHA256SUMS`](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/SHA256SUMS). Provenance : `gh attestation verify <fichier> --repo FantasmaGlad/Bonk`.

## Compiler

```bash
cargo build --workspace --release
```

[MIT](LICENSE). Un disque système n'est jamais proposé ni écrit.
