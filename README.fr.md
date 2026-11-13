# Bonk

<p align="center">
  <img src="crates/bonk/assets/icons/256x256/bonk.png" width="160" height="160" alt="Bonk">
</p>

<p align="center">
  <b>Écrit des images amorçables sur clé USB. Un seul binaire, ligne de commande et fenêtre, aucune télémétrie.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Téléchargements</b></a> &nbsp;·&nbsp;
  <a href="https://bonk-os.app"><b>Site</b></a> &nbsp;·&nbsp;
  <a href="https://bonk-os.app/reference/"><b>Commandes</b></a> &nbsp;·&nbsp;
  <a href="README.md"><b>English</b></a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Version&color=brightgreen" alt="Version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/Licence-MIT-blue.svg" alt="Licence: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI + GUI">
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Plateformes-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-success" alt="Plateformes : Linux, Windows, macOS, Android">
</p>

---

## Télécharger

| Plateforme | Téléchargement | État |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**Debian,&nbsp;Ubuntu** | [x86_64&nbsp;.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk_0.0.6-1_amd64.deb)&nbsp;·&nbsp;[ARM64&nbsp;.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk_0.0.6-1_arm64.deb) | Écrit les clés USB |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**Fedora,&nbsp;RHEL** | [x86_64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-1.x86_64.rpm)&nbsp;·&nbsp;[ARM64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-1.aarch64.rpm) | Écrit les clés USB |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**openSUSE,&nbsp;SLES** | [x86_64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-1.x86_64.rpm)&nbsp;·&nbsp;[ARM64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-1.aarch64.rpm) | Écrit les clés USB |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**Arch&nbsp;Linux** | [x86_64&nbsp;.pkg.tar.zst](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-1-x86_64.pkg.tar.zst) | Écrit les clés USB |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Tout&nbsp;Linux** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-linux-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-linux-aarch64.tar.gz)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-linux-i686.tar.gz) | Écrit les clés USB ; x86 non essayé |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-windows-x86_64-setup.exe)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-windows-aarch64-setup.exe)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-windows-i686-setup.exe) | Écrit les clés USB |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [Universal&nbsp;.dmg](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-macos-universal.dmg) | CI seulement, pas de vraie clé |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Android&nbsp;8+** | [ARM64&nbsp;.apk](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-android-arm64.apk)&nbsp;·&nbsp;[x86_64&nbsp;.apk](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-0.0.6-android-x86_64.apk) | Bêta, un téléphone essayé |

Mêmes paquets sur les dérivées : <img src="docs/icons/linuxmint.svg" width="16" height="16" valign="middle" alt="linuxmint" title="Linux Mint"> Linux Mint · <img src="docs/icons/popos.svg" width="16" height="16" valign="middle" alt="popos" title="Pop!_OS"> Pop!_OS · <img src="docs/icons/zorin.svg" width="16" height="16" valign="middle" alt="zorin" title="Zorin OS"> Zorin OS · <img src="docs/icons/raspberrypi.svg" width="16" height="16" valign="middle" alt="raspberrypi" title="Raspberry Pi OS"> Raspberry Pi OS · <img src="docs/icons/rockylinux.svg" width="16" height="16" valign="middle" alt="rockylinux" title="Rocky Linux"> Rocky Linux · <img src="docs/icons/almalinux.svg" width="16" height="16" valign="middle" alt="almalinux" title="AlmaLinux"> AlmaLinux · <img src="docs/icons/manjaro.svg" width="16" height="16" valign="middle" alt="manjaro" title="Manjaro"> Manjaro · <img src="docs/icons/endeavouros.svg" width="16" height="16" valign="middle" alt="endeavouros" title="EndeavourOS"> EndeavourOS · <img src="docs/icons/cachyos.svg" width="16" height="16" valign="middle" alt="cachyos" title="CachyOS"> CachyOS

Chromebook : dans l'environnement de développement Linux (Crostini, Debian), le paquet Debian ou le [dépôt APT](https://bonk-os.app/install/). Pas encore essayé sur un Chromebook.

Fichiers portables `.exe`, `.zip` et `.tar.gz` : [dernière version](https://github.com/FantasmaGlad/Bonk/releases/latest).

Empreintes : [`SHA256SUMS`](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/SHA256SUMS). Provenance : `gh attestation verify <fichier> --repo FantasmaGlad/Bonk`.

## Ligne de commande seule

Sans fenêtre ni bibliothèque graphique, pour les serveurs et les scripts. S'installe sans droits d'administrateur ; écrire sur une clé demande l'autorisation du système.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

```powershell
irm https://packages.bonk-os.app/install.ps1 | iex
```

Termux sous Android, sans root : l'application Termux:API de la même source que Termux (F-Droid ou GitHub), puis

```bash
pkg install termux-api
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

Chaque gestionnaire de paquets la propose aussi sous le nom `bonk-cli`, dans les [mêmes dépôts](https://bonk-os.app/install/) que `bonk`.

| Gestionnaire | Installer |
| :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**APT** | `sudo apt install bonk-cli` |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**DNF** | `sudo dnf install --allowerasing bonk-cli` |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**Zypper** | `sudo zypper install bonk-cli` |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**pacman** | `sudo pacman -S bonk-cli` |
| <img src="docs/icons/homebrew.svg" width="20" height="20" valign="middle" alt="homebrew">&nbsp;**Homebrew** | `brew uninstall bonk && brew install fantasmaglad/bonk/bonk-cli` |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Scoop** | `scoop uninstall bonk`, puis `scoop install` et l'adresse ci-dessous |

Scoop : `scoop install https://packages.bonk-os.app/scoop/bonk-cli.json`.

Avec APT, DNF, Zypper et pacman, installer `bonk-cli` remplace `bonk` (Zypper et pacman demandent d'abord) ; avec Homebrew et Scoop, retirer `bonk` d'abord. Pour revenir, retirer `bonk-cli`, puis installer `bonk`.

| Plateforme | Téléchargement | État |
| :--- | :--- | :--- |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Tout&nbsp;Linux** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-linux-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-linux-aarch64.tar.gz)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-linux-i686.tar.gz) | Écrit les clés USB |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-windows-x86_64.zip)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-windows-aarch64.zip)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-windows-i686.zip) | Écrit les clés USB |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-macos-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-macos-aarch64.tar.gz)&nbsp;·&nbsp;[Universal](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-macos-universal.tar.gz) | CI seulement, pas de vraie clé |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Termux** | [ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-android-aarch64.tar.gz)&nbsp;·&nbsp;[x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.6/bonk-cli-0.0.6-android-x86_64.tar.gz) | CI seulement, pas encore essayé sur un téléphone |
