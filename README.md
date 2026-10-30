# Bonk

Utilitaire de création de supports d'amorçage (clés USB et disques externes) pour Linux, Windows et macOS, écrit en Rust. Un seul exécutable : l'interface graphique sans argument, la ligne de commande sinon.

[![binaires](https://github.com/FantasmaGlad/Bonk/actions/workflows/binaires.yml/badge.svg)](https://github.com/FantasmaGlad/Bonk/actions/workflows/binaires.yml)
[![ci](https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml/badge.svg)](https://github.com/FantasmaGlad/Bonk/actions/workflows/ci.yml)

## Portée

Bonk écrit des images disque (`.img`, `.iso`, `.gz`, `.xz`, `.zst`) sur une clé USB ou un disque externe, puis vérifie l'écriture. Il fonctionne sur Linux, Windows et macOS, en x86_64, x86 et ARM64 selon le système, avec une interface graphique et une ligne de commande équivalentes. Un disque système n'est jamais proposé ni écrit.

## Télécharger

Binaires de la version [0.0.1](https://github.com/FantasmaGlad/Bonk/releases/tag/v0.0.1), construits par la CI et attestés. Empreintes : [SHA256SUMS](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/SHA256SUMS). Inventaire des composants : [bonk.cdx.json](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk.cdx.json).

| Système | Architecture | Binaire v0.0.1 | Système minimal |
|---|---|---|---|
| Linux | x86_64 | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-linux-x86_64.tar.gz), [deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk_0.0.1-1_amd64.deb), [rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-1.x86_64.rpm) | glibc 2.17 (noyau 3.2) |
| Linux | aarch64 | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-linux-aarch64.tar.gz), [deb](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk_0.0.1-1_arm64.deb), [rpm](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-1.aarch64.rpm) | glibc 2.17 (noyau 4.1) |
| Linux | i686 | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-linux-i686.tar.gz) | glibc 2.17 |
| Linux (statique, ligne de commande) | x86_64 | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-linux-x86_64-musl.tar.gz) | tout noyau 3.2+ |
| Linux (statique, ligne de commande) | aarch64 | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-linux-aarch64-musl.tar.gz) | tout noyau 4.1+ |
| Windows | x86_64 | [zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-windows-x86_64.zip) | Windows 10 |
| Windows | ARM64 | [zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-windows-aarch64.zip) | Windows 10 ARM64 |
| Windows | x86 (32 bits) | [zip](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-windows-i686.zip) | Windows 10 |
| macOS | Apple Silicon | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-macos-aarch64.tar.gz) | macOS 12 |
| macOS | Intel | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-macos-x86_64.tar.gz) | macOS 12 |
| macOS | universel | [tar.gz](https://github.com/FantasmaGlad/Bonk/releases/download/v0.0.1/bonk-0.0.1-macos-universal.tar.gz) | macOS 12 |

Les liens fonctionnent une fois la release publiée. Les binaires de chaque fusion sur `main` sont dans les [exécutions du workflow `binaires`](https://github.com/FantasmaGlad/Bonk/actions/workflows/binaires.yml).

Vérifier une archive :

```bash
gh attestation verify bonk-0.0.1-linux-x86_64.tar.gz --repo FantasmaGlad/Bonk
```

Les binaires ne sont pas encore signés Authenticode (Windows) ni notarisés (macOS) : SmartScreen et Gatekeeper peuvent avertir.

## Principes

- Sécurité des disques : un disque système de l'hôte n'est jamais proposé ni écrit.
- Aucune télémétrie : le réseau ne sert qu'au catalogue et aux téléchargements demandés.
- Aucune dépendance à installer par l'utilisateur.

## Compiler

```bash
cargo build --workspace --release
cargo test --workspace
```

La chaîne Rust (stable, avec rustfmt et clippy) est sélectionnée par `rust-toolchain.toml`.

## Architecture

Voir [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Licence

Licence MIT : voir [LICENSE](LICENSE).
