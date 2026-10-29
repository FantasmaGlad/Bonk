# Bonk

Utilitaire universel de création de supports d'amorçage (clés USB et disques externes) pour Windows, Linux et macOS, écrit en Rust. Un seul exécutable : l'interface graphique sans argument, la ligne de commande sinon.

## État

Fondations en cours (jalon J0). Le dépôt contient pour l'instant le workspace Cargo, les crates et leurs rôles, et la documentation d'architecture. `bonk --help` et `bonk --version` sont les seules commandes disponibles ; aucune écriture sur disque n'est encore implémentée.

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
