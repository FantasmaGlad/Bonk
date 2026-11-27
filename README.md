# Bonk

<p align="center">
  <img src="crates/bonk/assets/icons/256x256/bonk.png" width="160" height="160" alt="Bonk">
</p>

<p align="center">
  <b>Write bootable images to USB drives. One binary, command line and window, zero telemetry.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Downloads</b></a>,
  <a href="https://bonk-os.app"><b>Website</b></a>,
  <a href="https://bonk-os.app/commands/"><b>Commands</b></a>,
  <a href="README.fr.md"><b>Français</b></a>
</p>

<p align="center">
  <img src="docs/icons/gitlab.svg" width="16" height="16" valign="middle" alt="GitLab">&nbsp;Backup copy: <a href="https://gitlab.com/FantasmaGlad/Bonk">GitLab</a>, <a href="https://gitlab.com/FantasmaGlad/Bonk/-/releases">releases</a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Release&color=brightgreen" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI and GUI">
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Platforms-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-success" alt="Platforms: Linux, Windows, macOS, Android">
</p>

---

## Download

Bonk with its window and the `bonk` command; on Android, the app. On macOS, the `.dmg` holds the window app, the command line comes from Homebrew or the script; allow Bonk to access removable volumes when the system asks.

| Platform | Download | Status |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**Debian,&nbsp;Ubuntu** | [x86_64&nbsp;.deb](https://packages.bonk-os.app/download/bonk_amd64.deb),&nbsp;[ARM64&nbsp;.deb](https://packages.bonk-os.app/download/bonk_arm64.deb) | Writes USB drives |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**Fedora,&nbsp;RHEL** | [x86_64&nbsp;.rpm](https://packages.bonk-os.app/download/bonk.x86_64.rpm),&nbsp;[ARM64&nbsp;.rpm](https://packages.bonk-os.app/download/bonk.aarch64.rpm) | Writes USB drives |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**openSUSE,&nbsp;SLES** | [x86_64&nbsp;.rpm](https://packages.bonk-os.app/download/bonk.x86_64.rpm),&nbsp;[ARM64&nbsp;.rpm](https://packages.bonk-os.app/download/bonk.aarch64.rpm) | Writes USB drives |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**Arch&nbsp;Linux** | [x86_64&nbsp;.pkg.tar.zst](https://packages.bonk-os.app/download/bonk-x86_64.pkg.tar.zst) | Writes USB drives |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Any&nbsp;Linux** | [x86_64](https://packages.bonk-os.app/download/bonk-linux-x86_64.tar.gz),&nbsp;[ARM64](https://packages.bonk-os.app/download/bonk-linux-aarch64.tar.gz),&nbsp;[x86](https://packages.bonk-os.app/download/bonk-linux-i686.tar.gz) | Writes USB drives; x86 untested |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://packages.bonk-os.app/download/bonk-windows-x86_64-setup.exe),&nbsp;[ARM64](https://packages.bonk-os.app/download/bonk-windows-aarch64-setup.exe),&nbsp;[x86](https://packages.bonk-os.app/download/bonk-windows-i686-setup.exe) | Writes USB drives; ARM64 and x86 untested |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [Universal&nbsp;.dmg](https://packages.bonk-os.app/download/bonk-macos-universal.dmg) | Intel and Apple silicon (M1 and later) |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Android&nbsp;8+** | [ARM64&nbsp;.apk](https://packages.bonk-os.app/download/bonk-android-arm64.apk),&nbsp;[x86_64&nbsp;.apk](https://packages.bonk-os.app/download/bonk-android-x86_64.apk) | Writes USB drives |

## Command line only

No window and no graphics libraries, for servers and scripts. Installs without administrator rights; writing to a drive asks the system for authorization.

| Platform | Download | Status |
| :--- | :--- | :--- |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Any&nbsp;Linux** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-linux-x86_64.tar.gz),&nbsp;[ARM64](https://packages.bonk-os.app/download/bonk-cli-linux-aarch64.tar.gz),&nbsp;[x86](https://packages.bonk-os.app/download/bonk-cli-linux-i686.tar.gz) | Writes USB drives; x86 untested |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-windows-x86_64.zip),&nbsp;[ARM64](https://packages.bonk-os.app/download/bonk-cli-windows-aarch64.zip),&nbsp;[x86](https://packages.bonk-os.app/download/bonk-cli-windows-i686.zip) | Writes USB drives; ARM64 and x86 untested |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [x86_64](https://packages.bonk-os.app/download/bonk-cli-macos-x86_64.tar.gz),&nbsp;[ARM64](https://packages.bonk-os.app/download/bonk-cli-macos-aarch64.tar.gz),&nbsp;[Universal](https://packages.bonk-os.app/download/bonk-cli-macos-universal.tar.gz) | Intel and Apple silicon (M1 and later) |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Termux** | [ARM64](https://packages.bonk-os.app/download/bonk-cli-android-aarch64.tar.gz),&nbsp;[x86_64](https://packages.bonk-os.app/download/bonk-cli-android-x86_64.tar.gz) | Writes USB drives |

## Package managers

Add Bonk's signed repository first, as shown on [bonk-os.app/install](https://bonk-os.app/install/).

| Manager | Bonk | Command line only |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**APT** | `sudo apt install bonk` | `sudo apt install bonk-cli` |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**DNF** | `sudo dnf install bonk` | `sudo dnf install --allowerasing bonk-cli` |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**Zypper** | `sudo zypper install bonk` | `sudo zypper install bonk-cli` |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**pacman** | `sudo pacman -S bonk` | `sudo pacman -S bonk-cli` |
| <img src="docs/icons/homebrew.svg" width="20" height="20" valign="middle" alt="homebrew">&nbsp;**Homebrew** | `brew install fantasmaglad/bonk/bonk` | `brew install fantasmaglad/bonk/bonk-cli` |

Scoop, with `bonk-cli.json` for the command line only:

```powershell
scoop install https://packages.bonk-os.app/scoop/bonk.json
```

With APT, DNF (`--allowerasing`), Zypper and pacman, installing `bonk-cli` replaces `bonk` (Zypper and pacman ask first); with Homebrew and Scoop, remove `bonk` first. To go back, remove `bonk-cli`, then install `bonk`.

## Install

### <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="Linux"> <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="macOS"> Linux and macOS script

The command line only, in `~/.local/bin`, added to the shell's `PATH`; its checksum is checked against the release.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

### <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="Windows"> Windows

The command line only, for this user. Scoop installs either package from its manifest.

```powershell
irm https://packages.bonk-os.app/install.ps1 | iex
```

### <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="Android"> Termux

On Android, without root: the Termux:API app from the same source as Termux (F-Droid or GitHub). The script adds Bonk's repository to `pkg`, then `pkg upgrade` keeps it current.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

### Chromebook

In the Linux development environment (Crostini, Debian), the Debian package, the APT repository or the script. Enable the USB drive for Linux first, in the ChromeOS settings (Linux development environment, Manage USB devices); a drive goes to one environment at a time, so for the Android app share it with Google Play Store (Android apps) instead of Linux.

### Derivatives

The same packages and repositories serve <img src="docs/icons/linuxmint.svg" width="16" height="16" valign="middle" alt="linuxmint"> Linux Mint, <img src="docs/icons/popos.svg" width="16" height="16" valign="middle" alt="popos"> Pop!_OS, <img src="docs/icons/zorin.svg" width="16" height="16" valign="middle" alt="zorin"> Zorin OS, <img src="docs/icons/raspberrypi.svg" width="16" height="16" valign="middle" alt="raspberrypi"> Raspberry Pi OS, <img src="docs/icons/rockylinux.svg" width="16" height="16" valign="middle" alt="rockylinux"> Rocky Linux, <img src="docs/icons/almalinux.svg" width="16" height="16" valign="middle" alt="almalinux"> AlmaLinux, <img src="docs/icons/manjaro.svg" width="16" height="16" valign="middle" alt="manjaro"> Manjaro, <img src="docs/icons/endeavouros.svg" width="16" height="16" valign="middle" alt="endeavouros"> EndeavourOS, <img src="docs/icons/cachyos.svg" width="16" height="16" valign="middle" alt="cachyos"> CachyOS.

### Portable files

Portable `.exe`, `.zip` and `.tar.gz` files: [latest release](https://github.com/FantasmaGlad/Bonk/releases/latest).

### Checksums and provenance

Checksums: [`SHA256SUMS`](https://packages.bonk-os.app/download/SHA256SUMS). Provenance: `gh attestation verify <file> --repo FantasmaGlad/Bonk`.

## Vault

Several ISO images on one drive: `bonk vault init`, then `bonk vault add`. The vault starts in BIOS and UEFI with Secure Boot turned off; on a Windows PC encrypted with BitLocker, suspend BitLocker before changing Secure Boot.

## Related repositories

| Repository | Content |
| :--- | :--- |
| <img src="crates/bonk/assets/icons/32x32/bonk.png" width="20" height="20" valign="middle" alt="BonkWeb">&nbsp;**[BonkWeb](https://github.com/FantasmaGlad/BonkWeb)** | website, [bonk-os.app](https://bonk-os.app) |
| <img src="crates/bonk/assets/icons/32x32/bonk.png" width="20" height="20" valign="middle" alt="bonk-catalog">&nbsp;**[bonk-catalog](https://github.com/FantasmaGlad/bonk-catalog)** | image catalog, [catalog.bonk-os.app](https://catalog.bonk-os.app) |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="bonk-packages">&nbsp;**[bonk-packages](https://github.com/FantasmaGlad/bonk-packages)** | package repositories, [packages.bonk-os.app](https://packages.bonk-os.app) |
| <img src="docs/icons/homebrew.svg" width="20" height="20" valign="middle" alt="homebrew-bonk">&nbsp;**[homebrew-bonk](https://github.com/FantasmaGlad/homebrew-bonk)** | Homebrew tap, [bonk-os.app/install](https://bonk-os.app/install/) |
| <img src="crates/bonk/assets/icons/32x32/bonk.png" width="20" height="20" valign="middle" alt="bonk-security">&nbsp;**[bonk-security](https://github.com/FantasmaGlad/bonk-security)** | security site, [security.bonk-os.app](https://security.bonk-os.app) |
