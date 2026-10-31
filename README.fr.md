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
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Plateforme-Linux-success?logo=linux&logoColor=white" alt="Plateforme: Linux">
  <img src="https://img.shields.io/badge/Preview-Windows%20%7C%20macOS-yellow" alt="Preview: Windows, macOS">
</p>

---

## Télécharger

| Famille | x86_64 | ARM64 | x86 | État |
| :--- | :---: | :---: | :---: | :--- |
| <img src="docs/icons/debian.svg" width="18" height="18" valign="middle" alt="debian"> <img src="docs/icons/ubuntu.svg" width="18" height="18" valign="middle" alt="ubuntu"> <img src="docs/icons/linuxmint.svg" width="18" height="18" valign="middle" alt="linuxmint"> <img src="docs/icons/popos.svg" width="18" height="18" valign="middle" alt="popos"> <img src="docs/icons/zorin.svg" width="18" height="18" valign="middle" alt="zorin"> **Debian, Ubuntu, Mint, Pop!_OS, Zorin** | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk_0.0.3-1_amd64.deb) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-x86_64.tar.gz) | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk_0.0.3-1_arm64.deb) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-aarch64.tar.gz) | — | Écrit les clés USB |
| <img src="docs/icons/raspberrypi.svg" width="18" height="18" valign="middle" alt="raspberrypi"> **Raspberry Pi OS** | — | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk_0.0.3-1_arm64.deb) | — | Écrit les clés USB |
| <img src="docs/icons/fedora.svg" width="18" height="18" valign="middle" alt="fedora"> <img src="docs/icons/rhel.svg" width="18" height="18" valign="middle" alt="rhel"> <img src="docs/icons/rockylinux.svg" width="18" height="18" valign="middle" alt="rockylinux"> <img src="docs/icons/almalinux.svg" width="18" height="18" valign="middle" alt="almalinux"> **Fedora, RHEL, Rocky, Alma** | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-1.x86_64.rpm) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-x86_64.tar.gz) | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-1.aarch64.rpm) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-aarch64.tar.gz) | — | Écrit les clés USB |
| <img src="docs/icons/opensuse.svg" width="18" height="18" valign="middle" alt="opensuse"> **openSUSE, SLES** | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-1.x86_64.rpm) | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-1.aarch64.rpm) | — | Écrit les clés USB |
| <img src="docs/icons/arch.svg" width="18" height="18" valign="middle" alt="arch"> <img src="docs/icons/manjaro.svg" width="18" height="18" valign="middle" alt="manjaro"> <img src="docs/icons/endeavouros.svg" width="18" height="18" valign="middle" alt="endeavouros"> <img src="docs/icons/cachyos.svg" width="18" height="18" valign="middle" alt="cachyos"> **Arch, Manjaro, EndeavourOS, CachyOS** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-aarch64.tar.gz) | — | Écrit les clés USB |
| <img src="docs/icons/linux.svg" width="18" height="18" valign="middle" alt="linux"> **Tout Linux** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-aarch64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-i686.tar.gz) | Écrit les clés USB |
| <img src="docs/icons/linux.svg" width="18" height="18" valign="middle" alt="linux"> **Tout Linux, statique, CLI seule** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-x86_64-musl.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-linux-aarch64-musl.tar.gz) | — | Écrit les clés USB |
| <img src="docs/icons/windows.svg" width="18" height="18" valign="middle" alt="windows"> **Windows 10+** | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-x86_64.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-x86_64.zip) | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-aarch64.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-aarch64.zip) | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-i686.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-windows-i686.zip) | Aperçu : pas encore d'accès aux disques |
| <img src="docs/icons/windows.svg" width="18" height="18" valign="middle" alt="windows"> **Windows 10+, CLI seule** | [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-cli-0.0.3-windows-x86_64.zip) | [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-cli-0.0.3-windows-aarch64.zip) | [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-cli-0.0.3-windows-i686.zip) | Aperçu : pas encore d'accès aux disques |
| <img src="docs/icons/apple.svg" width="18" height="18" valign="middle" alt="apple"> **macOS 12+** | [.dmg](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-macos-universal.dmg) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-macos-x86_64.tar.gz) | [.dmg](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-macos-universal.dmg) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-0.0.3-macos-aarch64.tar.gz) | — | Aperçu : pas encore d'accès aux disques |
| <img src="docs/icons/apple.svg" width="18" height="18" valign="middle" alt="apple"> **macOS 12+, CLI seule** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-cli-0.0.3-macos-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/bonk-cli-0.0.3-macos-aarch64.tar.gz) | — | Aperçu : pas encore d'accès aux disques |

Empreintes : [`SHA256SUMS`](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.3/SHA256SUMS). Provenance : `gh attestation verify <fichier> --repo FantasmaGlad/Bonk`.
