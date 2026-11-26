# Bonk

<p align="center">
  <img src="crates/bonk/assets/icons/256x256/bonk.png" width="160" height="160" alt="Bonk">
</p>

<p align="center">
  <b>Écrit des images amorçables sur clé USB. Un seul binaire, ligne de commande et fenêtre, aucune télémétrie.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Téléchargements</b></a>,
  <a href="https://bonk-os.app"><b>Site</b></a>,
  <a href="https://bonk-os.app/commands/"><b>Commandes</b></a>,
  <a href="README.md"><b>English</b></a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Version&color=brightgreen" alt="Version"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/Licence-MIT-blue.svg" alt="Licence: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI et interface graphique">
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Plateformes-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-success" alt="Plateformes : Linux, Windows, macOS, Android">
</p>

---

## Télécharger

Bonk avec sa fenêtre et la commande `bonk` ; sur Android, l'application. Sous macOS, l'application ne met pas `bonk` dans le Terminal : Homebrew ou le script ci-dessous.

| Plateforme | Téléchargement | État |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**Debian,&nbsp;Ubuntu** | [x86_64 .deb](https://packages.bonk-os.app/download/bonk_amd64.deb), [ARM64 .deb](https://packages.bonk-os.app/download/bonk_arm64.deb) | Écrit les clés USB |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**Fedora,&nbsp;RHEL** | [x86_64 .rpm](https://packages.bonk-os.app/download/bonk.x86_64.rpm), [ARM64 .rpm](https://packages.bonk-os.app/download/bonk.aarch64.rpm) | Écrit les clés USB |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**openSUSE,&nbsp;SLES** | [x86_64 .rpm](https://packages.bonk-os.app/download/bonk.x86_64.rpm), [ARM64 .rpm](https://packages.bonk-os.app/download/bonk.aarch64.rpm) | Écrit les clés USB |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**Arch&nbsp;Linux** | [x86_64 .pkg.tar.zst](https://packages.bonk-os.app/download/bonk-x86_64.pkg.tar.zst) | Écrit les clés USB |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Tout&nbsp;Linux** | [x86_64](https://packages.bonk-os.app/download/bonk-linux-x86_64.tar.gz), [ARM64](https://packages.bonk-os.app/download/bonk-linux-aarch64.tar.gz), [x86](https://packages.bonk-os.app/download/bonk-linux-i686.tar.gz) | Écrit les clés USB ; x86 non essayé |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://packages.bonk-os.app/download/bonk-windows-x86_64-setup.exe), [ARM64](https://packages.bonk-os.app/download/bonk-windows-aarch64-setup.exe), [x86](https://packages.bonk-os.app/download/bonk-windows-i686-setup.exe) | Écrit les clés USB ; ARM64 et x86 non essayés |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [Universal .dmg](https://packages.bonk-os.app/download/bonk-macos-universal.dmg) | Lancée sur un Mac ; écriture pas encore essayée |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Android&nbsp;8+** | [ARM64 .apk](https://packages.bonk-os.app/download/bonk-android-arm64.apk), [x86_64 .apk](https://packages.bonk-os.app/download/bonk-android-x86_64.apk) | Essayée sur cinq téléphones |

## Ligne de commande seule

Sans fenêtre ni bibliothèque graphique, pour les serveurs et les scripts. S'installe sans droits d'administrateur ; écrire sur une clé demande l'autorisation du système.

| Plateforme | Téléchargement | État |
| :--- | :--- | :--- |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Tout&nbsp;Linux** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-linux-x86_64.tar.gz), [ARM64](https://packages.bonk-os.app/download/bonk-cli-linux-aarch64.tar.gz), [x86](https://packages.bonk-os.app/download/bonk-cli-linux-i686.tar.gz) | Écrit les clés USB ; x86 non essayé |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-windows-x86_64.zip), [ARM64](https://packages.bonk-os.app/download/bonk-cli-windows-aarch64.zip), [x86](https://packages.bonk-os.app/download/bonk-cli-windows-i686.zip) | Écrit les clés USB ; ARM64 et x86 non essayés |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-macos-x86_64.tar.gz), [ARM64](https://packages.bonk-os.app/download/bonk-cli-macos-aarch64.tar.gz), [Universal](https://packages.bonk-os.app/download/bonk-cli-macos-universal.tar.gz) | CI seulement, sans vraie clé |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Termux** | [ARM64](https://packages.bonk-os.app/download/bonk-cli-android-aarch64.tar.gz), [x86_64](https://packages.bonk-os.app/download/bonk-cli-android-x86_64.tar.gz) | CI seulement, jamais sur un téléphone |

## Gestionnaires de paquets

Ajouter d'abord le dépôt signé de Bonk, comme indiqué sur [bonk-os.app/install](https://bonk-os.app/install/).

| Gestionnaire | Bonk | Ligne de commande seule |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**APT** | `sudo apt install bonk` | `sudo apt install bonk-cli` |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**DNF** | `sudo dnf install bonk` | `sudo dnf install --allowerasing bonk-cli` |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**Zypper** | `sudo zypper install bonk` | `sudo zypper install bonk-cli` |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**pacman** | `sudo pacman -S bonk` | `sudo pacman -S bonk-cli` |
| <img src="docs/icons/homebrew.svg" width="20" height="20" valign="middle" alt="homebrew">&nbsp;**Homebrew** | `brew install fantasmaglad/bonk/bonk` | `brew install fantasmaglad/bonk/bonk-cli` |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Scoop** | `scoop install` [bonk.json](https://packages.bonk-os.app/scoop/bonk.json) | `scoop install` [bonk-cli.json](https://packages.bonk-os.app/scoop/bonk-cli.json) |

Avec APT, DNF (`--allowerasing`), Zypper et pacman, installer `bonk-cli` remplace `bonk` (Zypper et pacman demandent d'abord) ; avec Homebrew et Scoop, retirer `bonk` d'abord. Pour revenir, retirer `bonk-cli`, puis installer `bonk`.

## Installer

### <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="Linux"> <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="macOS"> Script Linux et macOS

La ligne de commande seule, dans `~/.local/bin`, son empreinte contrôlée contre la version publiée.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

### <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="Windows"> Windows

La ligne de commande seule, pour cet utilisateur. Scoop installe l'un ou l'autre paquet depuis son manifeste.

```powershell
irm https://packages.bonk-os.app/install.ps1 | iex
```

### <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="Android"> Termux

Sous Android, sans root : l'application Termux:API de la même source que Termux (F-Droid ou GitHub). Le script ajoute le dépôt de Bonk à `pkg`, puis `pkg upgrade` le tient à jour.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

### Chromebook

Dans l'environnement de développement Linux (Crostini, Debian), le paquet Debian, le dépôt APT ou le script. Activer d'abord la clé USB pour Linux dans les paramètres de ChromeOS (environnement de développement Linux, Gérer les périphériques USB) ; pour l'application Android, l'activer plutôt pour Google Play Store.

### Dérivées

Les mêmes paquets et dépôts servent <img src="docs/icons/linuxmint.svg" width="16" height="16" valign="middle" alt="linuxmint"> Linux Mint, <img src="docs/icons/popos.svg" width="16" height="16" valign="middle" alt="popos"> Pop!_OS, <img src="docs/icons/zorin.svg" width="16" height="16" valign="middle" alt="zorin"> Zorin OS, <img src="docs/icons/raspberrypi.svg" width="16" height="16" valign="middle" alt="raspberrypi"> Raspberry Pi OS, <img src="docs/icons/rockylinux.svg" width="16" height="16" valign="middle" alt="rockylinux"> Rocky Linux, <img src="docs/icons/almalinux.svg" width="16" height="16" valign="middle" alt="almalinux"> AlmaLinux, <img src="docs/icons/manjaro.svg" width="16" height="16" valign="middle" alt="manjaro"> Manjaro, <img src="docs/icons/endeavouros.svg" width="16" height="16" valign="middle" alt="endeavouros"> EndeavourOS, <img src="docs/icons/cachyos.svg" width="16" height="16" valign="middle" alt="cachyos"> CachyOS.

### Fichiers portables

Fichiers portables `.exe`, `.zip` et `.tar.gz` : [dernière version](https://github.com/FantasmaGlad/Bonk/releases/latest).

### Empreintes et provenance

Empreintes : [`SHA256SUMS`](https://packages.bonk-os.app/download/SHA256SUMS). Provenance : `gh attestation verify <fichier> --repo FantasmaGlad/Bonk`.

## Coffre

Plusieurs images ISO sur une clé : `bonk vault init`, puis `bonk vault add`. Le coffre démarre en BIOS et en UEFI, Secure Boot désactivé ; sur un PC Windows chiffré par BitLocker, suspendre BitLocker avant de changer Secure Boot.
