# Bonk

<p align="center">
  <b>Write bootable images to USB drives. One binary, command line and window, zero telemetry.</b>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><b>Downloads</b></a> &nbsp;·&nbsp;
  <a href="README.fr.md"><b>Français</b></a> &nbsp;·&nbsp;
  <a href="docs/ARCHITECTURE.md"><b>Architecture</b></a>
</p>

<p align="center">
  <a href="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml"><img src="https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/FantasmaGlad/Bonk/releases/latest"><img src="https://img.shields.io/github/v/release/FantasmaGlad/Bonk?label=Release&color=brightgreen" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/CLI-GUI-lightgrey" alt="CLI + GUI">
  <img src="https://img.shields.io/badge/Rust-1.89%2B-orange?logo=rust&logoColor=white" alt="Rust 1.89+">
  <img src="https://img.shields.io/badge/Platform-Linux-success?logo=linux&logoColor=white" alt="Platform: Linux">
  <img src="https://img.shields.io/badge/Preview-Windows%20%7C%20macOS-yellow" alt="Preview: Windows, macOS">
</p>

---

## Get started

```bash
tar xzf bonk-0.0.2-linux-x86_64.tar.gz
./bonk list
./bonk write <id> https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/debian-13.7.0-amd64-netinst.iso \
  --sha256-file https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/SHA256SUMS --yes
```

`bonk write` takes a URL or a local file (`.iso`, `.img`, `.gz`, `.xz`, `.zst`), verifies it, writes it and reads it back. A Windows 10 or 11 x86_64 ISO becomes a UEFI install drive. No `sudo`: Linux asks for permission through polkit. Run `bonk` alone to open the window.

## Download

| Family | x86_64 | ARM64 | x86 | Status |
| :--- | :---: | :---: | :---: | :--- |
| <img src="docs/icons/debian.svg" width="18" height="18" valign="middle" alt="debian"> <img src="docs/icons/ubuntu.svg" width="18" height="18" valign="middle" alt="ubuntu"> <img src="docs/icons/linuxmint.svg" width="18" height="18" valign="middle" alt="linuxmint"> <img src="docs/icons/popos.svg" width="18" height="18" valign="middle" alt="popos"> <img src="docs/icons/zorin.svg" width="18" height="18" valign="middle" alt="zorin"> **Debian, Ubuntu, Mint, Pop!_OS, Zorin** | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk_0.0.2-1_amd64.deb) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64.tar.gz) | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk_0.0.2-1_arm64.deb) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64.tar.gz) | — | Writes USB drives |
| <img src="docs/icons/raspberrypi.svg" width="18" height="18" valign="middle" alt="raspberrypi"> **Raspberry Pi OS** | — | [.deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk_0.0.2-1_arm64.deb) | — | Writes USB drives |
| <img src="docs/icons/fedora.svg" width="18" height="18" valign="middle" alt="fedora"> <img src="docs/icons/rhel.svg" width="18" height="18" valign="middle" alt="rhel"> <img src="docs/icons/rockylinux.svg" width="18" height="18" valign="middle" alt="rockylinux"> <img src="docs/icons/almalinux.svg" width="18" height="18" valign="middle" alt="almalinux"> **Fedora, RHEL, Rocky, Alma** | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.x86_64.rpm) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64.tar.gz) | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.aarch64.rpm) · [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64.tar.gz) | — | Writes USB drives |
| <img src="docs/icons/opensuse.svg" width="18" height="18" valign="middle" alt="opensuse"> **openSUSE, SLES** | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.x86_64.rpm) | [.rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-1.aarch64.rpm) | — | Writes USB drives |
| <img src="docs/icons/arch.svg" width="18" height="18" valign="middle" alt="arch"> <img src="docs/icons/manjaro.svg" width="18" height="18" valign="middle" alt="manjaro"> <img src="docs/icons/endeavouros.svg" width="18" height="18" valign="middle" alt="endeavouros"> <img src="docs/icons/cachyos.svg" width="18" height="18" valign="middle" alt="cachyos"> **Arch, Manjaro, EndeavourOS, CachyOS** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64.tar.gz) | — | Writes USB drives |
| <img src="docs/icons/linux.svg" width="18" height="18" valign="middle" alt="linux"> **Any Linux** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-i686.tar.gz) | Writes USB drives |
| <img src="docs/icons/linux.svg" width="18" height="18" valign="middle" alt="linux"> **Any Linux, static, CLI only** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-x86_64-musl.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-linux-aarch64-musl.tar.gz) | — | Writes USB drives |
| <img src="docs/icons/windows.svg" width="18" height="18" valign="middle" alt="windows"> **Windows 10+** | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-x86_64.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-x86_64.zip) | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-aarch64.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-aarch64.zip) | [.exe](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-i686.exe) · [.zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-windows-i686.zip) | Preview: no disk access yet |
| <img src="docs/icons/apple.svg" width="18" height="18" valign="middle" alt="apple"> **macOS 12+** | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-macos-x86_64.tar.gz) | [.tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/bonk-0.0.2-macos-aarch64.tar.gz) | — | Preview: no disk access yet |

Checksums: [`SHA256SUMS`](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.2/SHA256SUMS). Provenance: `gh attestation verify <file> --repo FantasmaGlad/Bonk`.

## Build

```bash
cargo build --workspace --release
```

[MIT](LICENSE). A system disk is never offered or written.
