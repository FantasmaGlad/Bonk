# Bonk

<p align="center">
  <b>Write bootable images to USB drives. One binary, command line and window, zero telemetry.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Downloads</b></a> &nbsp;·&nbsp;
  <a href="https://bonk-os.app"><b>Website</b></a> &nbsp;·&nbsp;
  <a href="README.fr.md"><b>Français</b></a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Release&color=brightgreen" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI + GUI">
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Platforms-Linux%20%7C%20Windows%20%7C%20macOS%20%7C%20Android-success" alt="Platforms: Linux, Windows, macOS, Android">
</p>

---

## Download

| Platform | Download | Status |
| :--- | :--- | :--- |
| <img src="docs/icons/debian.svg" width="20" height="20" valign="middle" alt="debian">&nbsp;**Debian,&nbsp;Ubuntu** | [x86_64&nbsp;.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk_0.0.5-1_amd64.deb)&nbsp;·&nbsp;[ARM64&nbsp;.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk_0.0.5-1_arm64.deb) | Writes USB drives |
| <img src="docs/icons/fedora.svg" width="20" height="20" valign="middle" alt="fedora">&nbsp;**Fedora,&nbsp;RHEL** | [x86_64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-1.x86_64.rpm)&nbsp;·&nbsp;[ARM64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-1.aarch64.rpm) | Writes USB drives |
| <img src="docs/icons/opensuse.svg" width="20" height="20" valign="middle" alt="opensuse">&nbsp;**openSUSE,&nbsp;SLES** | [x86_64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-1.x86_64.rpm)&nbsp;·&nbsp;[ARM64&nbsp;.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-1.aarch64.rpm) | Writes USB drives |
| <img src="docs/icons/arch.svg" width="20" height="20" valign="middle" alt="arch">&nbsp;**Arch&nbsp;Linux** | [x86_64&nbsp;.pkg.tar.zst](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-1-x86_64.pkg.tar.zst) | Writes USB drives |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Any&nbsp;Linux** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-linux-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-linux-aarch64.tar.gz)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-linux-i686.tar.gz) | Writes USB drives, x86 built but never run |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-windows-x86_64-setup.exe)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-windows-aarch64-setup.exe)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-windows-i686-setup.exe) | Proven in CI, not yet on a real drive |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [Universal&nbsp;.dmg](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-macos-universal.dmg) | Proven in CI, not yet on a real drive |
| <img src="docs/icons/android.svg" width="20" height="20" valign="middle" alt="android">&nbsp;**Android&nbsp;8+** | [ARM64&nbsp;.apk](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-0.0.5-android-arm64.apk) | Beta, tried on one phone (Android 16), USB adapter, no root |

Same packages on derivatives: <img src="docs/icons/linuxmint.svg" width="16" height="16" valign="middle" alt="linuxmint" title="Linux Mint"> Linux Mint · <img src="docs/icons/popos.svg" width="16" height="16" valign="middle" alt="popos" title="Pop!_OS"> Pop!_OS · <img src="docs/icons/zorin.svg" width="16" height="16" valign="middle" alt="zorin" title="Zorin OS"> Zorin OS · <img src="docs/icons/raspberrypi.svg" width="16" height="16" valign="middle" alt="raspberrypi" title="Raspberry Pi OS"> Raspberry Pi OS · <img src="docs/icons/rockylinux.svg" width="16" height="16" valign="middle" alt="rockylinux" title="Rocky Linux"> Rocky Linux · <img src="docs/icons/almalinux.svg" width="16" height="16" valign="middle" alt="almalinux" title="AlmaLinux"> AlmaLinux · <img src="docs/icons/manjaro.svg" width="16" height="16" valign="middle" alt="manjaro" title="Manjaro"> Manjaro · <img src="docs/icons/endeavouros.svg" width="16" height="16" valign="middle" alt="endeavouros" title="EndeavourOS"> EndeavourOS · <img src="docs/icons/cachyos.svg" width="16" height="16" valign="middle" alt="cachyos" title="CachyOS"> CachyOS

Portable `.exe`, `.zip` and `.tar.gz` files: [latest release](https://github.com/FantasmaGlad/Bonk/releases/latest).

Checksums: [`SHA256SUMS`](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/SHA256SUMS). Provenance: `gh attestation verify <file> --repo FantasmaGlad/Bonk`.

## Command line only

No window and no graphics libraries, for servers and scripts. No administrator rights needed.

```bash
curl -fsSL https://packages.bonk-os.app/install.sh | sh
```

```powershell
irm https://packages.bonk-os.app/install.ps1 | iex
```

Every package manager also offers it as `bonk-cli`: [commands](https://bonk-os.app/install/).

| Platform | Download | Status |
| :--- | :--- | :--- |
| <img src="docs/icons/linux.svg" width="20" height="20" valign="middle" alt="linux">&nbsp;**Any&nbsp;Linux** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-linux-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-linux-aarch64.tar.gz) | Writes USB drives |
| <img src="docs/icons/windows.svg" width="20" height="20" valign="middle" alt="windows">&nbsp;**Windows&nbsp;10+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-windows-x86_64.zip)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-windows-aarch64.zip)&nbsp;·&nbsp;[x86](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-windows-i686.zip) | Not yet tried on a real drive |
| <img src="docs/icons/apple.svg" width="20" height="20" valign="middle" alt="apple">&nbsp;**macOS&nbsp;12+** | [x86_64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-macos-x86_64.tar.gz)&nbsp;·&nbsp;[ARM64](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.5/bonk-cli-0.0.5-macos-aarch64.tar.gz) | Not yet tried on a real drive |
