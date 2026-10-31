# Bonk

Écrit des images amorçables sur clé USB ou disque externe, en ligne de commande sur toute plateforme, avec une fenêtre quand c'est possible. Écrit en Rust, un seul binaire, licence [MIT](LICENSE).

[![binaires](https://github.com/FantasmaGlad/Bonk/actions/workflows/binaires.yml/badge.svg)](https://github.com/FantasmaGlad/Bonk/actions/workflows/binaires.yml)
[![ci](https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg)](https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml)

## État

Version 0.0.1, en développement, aucune release publiée.

- **Fait, Linux** : liste des disques, écriture, vérification, réparation du GPT, décompression gzip, xz, zstd, service `bonk serve`, contrôle d'empreinte et analyse d'image, écriture sans `sudo` (UDisks2 et polkit), formatage (`bonk format`, `bonk capabilities`).
- **Téléchargement** : `bonk write <clé> <URL> --sha256-file <URL de SHA256SUMS> --yes` télécharge l'image dans le cache, reprend après une coupure et contrôle l'empreinte avant d'écrire.
- **À venir** : Windows et macOS (binaires construits, sans accès aux disques), clé d'installation Windows, DOS, Android.

## Plateformes

| <sub>Famille</sub> | <sub>Systèmes</sub> | <sub>Puces</sub> | <sub>Fichier prévu</sub> | <sub>Lien</sub> |
|---|---|---|---|---|
| <sub>Debian</sub> | <sub>Debian, Ubuntu, Kubuntu, Xubuntu, Mint, Pop!_OS, Kali, Zorin, MX, elementary, Tuxedo, Deepin, Devuan, Proxmox VE, Tails, Raspberry Pi OS</sub> | <sub>x86_64, ARM64</sub> | <sub>`.deb`, `linux-<puce>.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Red Hat</sub> | <sub>Fedora, RHEL, CentOS, Rocky, AlmaLinux, Oracle Linux, Amazon Linux, Nobara</sub> | <sub>x86_64, ARM64</sub> | <sub>`.rpm`, `linux-<puce>.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>SUSE</sub> | <sub>SLES, openSUSE Leap, Tumbleweed</sub> | <sub>x86_64, ARM64</sub> | <sub>`.rpm`, `linux-<puce>.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Arch</sub> | <sub>Arch, CachyOS, EndeavourOS, Manjaro, Garuda</sub> | <sub>x86_64, ARM64</sub> | <sub>AUR, `linux-<puce>.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Linux immuable</sub> | <sub>SteamOS, Bazzite</sub> | <sub>x86_64, ARM64</sub> | <sub>AppImage, Flatpak</sub> | <sub>à venir</sub> |
| <sub>Linux 32 bits</sub> | <sub>Debian, Kali, MX, Devuan, Slackware, Arch Linux 32, LMDE</sub> | <sub>x86</sub> | <sub>`linux-i686.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Linux ARM 32 bits</sub> | <sub>Raspberry Pi OS 32 bits, Debian, Kali, Arch Linux ARM</sub> | <sub>ARMv7</sub> | <sub>`linux-armv7.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Indépendantes</sub> | <sub>Slackware, Gentoo, Void</sub> | <sub>x86_64, ARM64</sub> | <sub>`linux-<puce>.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Statique, CLI</sub> | <sub>Alpine, NixOS, Void musl, Gentoo musl, serveurs, tout Linux</sub> | <sub>x86_64, ARM64, x86, ARMv6</sub> | <sub>`linux-<puce>-musl.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Statique, CLI</sub> | <sub>Debian, Ubuntu, Fedora, openSUSE, RHEL, Deepin, Gentoo</sub> | <sub>RISC-V, POWER, LoongArch</sub> | <sub>`linux-<puce>-musl.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>ChromeOS</sub> | <sub>ChromeOS par Crostini, CLI ; accès à la clé à vérifier</sub> | <sub>x86_64, ARM64</sub> | <sub>`linux-<puce>-musl.tar.gz`</sub> | <sub>à venir</sub> |
| <sub>Windows</sub> | <sub>Windows 10, 11, Server 2016 et plus</sub> | <sub>x86_64, ARM64, x86</sub> | <sub>`windows-<puce>.zip`, MSIX</sub> | <sub>à venir</sub> |
| <sub>macOS</sub> | <sub>macOS 12 et plus</sub> | <sub>Apple Silicon, Intel</sub> | <sub>`macos-universal.tar.gz`, `.dmg`</sub> | <sub>à venir</sub> |
| <sub>Android</sub> | <sub>Android 8.0 et plus, téléphones et tablettes, Bliss OS, Android-x86, ChromeOS</sub> | <sub>ARM64, ARMv7, x86_64, x86</sub> | <sub>APK, Google Play</sub> | <sub>à venir</sub> |

<sub>Noms complets : `bonk-<version>-<cible>.<format>`. Empreintes `SHA256SUMS`, inventaire `bonk.cdx.json`, provenance vérifiable : `gh attestation verify <fichier> --repo FantasmaGlad/Bonk`. Pas de signature Authenticode ni de notarisation : SmartScreen et Gatekeeper peuvent avertir.</sub>

## Principes

- Un disque système n'est jamais proposé ni écrit.
- Aucune télémétrie.
- Aucune dépendance obligatoire. Sous Linux, l'écriture sans `sudo` passe par UDisks2 et polkit, présents sur la plupart des bureaux ; exFAT, NTFS et ext4 sont formatés par les outils du système.

## Compiler

```bash
cargo build --workspace --release
cargo test --workspace
```

Architecture : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
