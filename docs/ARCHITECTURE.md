# Architecture de Bonk

> Statut : `bonk-catalog` et `bonk-customizer` sont encore vides ; seul Linux a un accès aux disques ; l'élévation des privilèges (section Privilèges) reste à faire.

## Vue d'ensemble

```
              crates/bonk (binaire unique : fenêtre sans argument, ligne de commande sinon)
                 │                                  │
             bonk-gui                           bonk-cli
        (lance `bonk write`)          (arguments, texte et JSON, service)
                 │                                  │
                 │                              bonk-ops
                 │                  (porte d'écriture unique, codes d'erreur)
                 │                                  │
                 └──────────► bonk-hal ◄────────────┤
                       (filtre, trait Platform)     │
                                 │                  │
                                 └─► bonk-core ◄────┘
                             (pur : MBR/GPT, écriture, vérification)
```

| Crate | Rôle |
|---|---|
| `bonk` | Seul binaire. Choisit l'interface ; sous-mode `--helper` pour les opérations disque élevées. |
| `bonk-core` | Partitionnement MBR/GPT, écriture brute, décompression, vérification, réparation GPT. N'opère que sur une abstraction de périphérique bloc : aucune API d'OS. |
| `bonk-hal` | Filtre de sécurité et trait `Platform` : énumération, accès brut, démontage, par système. Seul crate contenant du code spécifique à un OS. |
| `bonk-ops` | Porte d'écriture unique (filtre, identité, démontage, vérification) et codes d'erreur stables. Sans interface : ni clap, ni Iced. |
| `bonk-customizer` | Fichiers de réponse Windows, persistance des systèmes Live Linux. |
| `bonk-catalog` | Manifeste signé, bibliothèque locale, téléchargements reprenables, empreintes. |
| `bonk-cli` | Commandes, sortie texte et JSON, codes de sortie stables. |
| `bonk-gui` | Interface graphique Iced. |

## Règles de dépendance
- `bonk-core` ne dépend d'aucun autre crate du workspace ni d'aucune API système.
- `bonk-ops` ne dépend d'aucune interface.
- `bonk-cli` et `bonk-gui` ne dépendent pas l'un de l'autre.
- Le code propre à un OS est isolé dans `bonk-hal` derrière le trait `Platform`.

Ces règles sont vérifiées par `crates/bonk/tests/architecture.rs`.

## Sécurité des disques
1. Les disques système de l'hôte (racine, démarrage, ESP active, volumes montés par le système, membres RAID/LVM, volumes chiffrés) ne sont jamais listés.
2. Par défaut, seuls les disques sur bus amovible (USB, SD, Thunderbolt externe) sont proposés ; le filtrage se fait sur le bus, pas sur le drapeau « amovible ».
3. L'identité du disque (numéro de série et taille) est revérifiée juste avant l'écriture.
4. En ligne de commande : cible désignée par un identifiant stable, `--yes` obligatoire, `--dry-run` disponible.

## Privilèges
L'interface ne s'exécute jamais avec des droits élevés. Seules les opérations disque le sont :

| OS | Mécanisme |
|---|---|
| Linux | UDisks2 et polkit |
| Windows | sous-processus `bonk --helper` élevé par UAC |
| macOS | `authopen` et DiskArbitration |

## Tests
- `bonk-core` est testé sur des fichiers image.
- Le filtre de sécurité est testé sur des jeux d'énumération capturés sur de vraies machines.
- Les supports produits sont démarrés dans QEMU avec un micrologiciel UEFI EDK2 (OVMF pour x86_64, AAVMF pour aarch64), avec et sans Secure Boot.
- Aucun test n'écrit sur un périphérique réel.
