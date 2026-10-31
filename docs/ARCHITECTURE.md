# Architecture de Bonk

> Statut : `bonk-customizer` est encore vide, `bonk-catalog` ne fait que le téléchargement ; seul Linux a un accès aux disques, et l'élévation des privilèges n'y est faite que pour lui.

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
| `bonk` | Seul binaire. Choisit l'interface. |
| `bonk-core` | Tables MBR/GPT, formatage FAT32, écriture brute, décompression, vérification, réparation GPT, analyse d'image. N'opère que sur une abstraction de périphérique bloc : aucune API d'OS. Lecture UDF, découpage WIM, arbre FAT32 et certificat de chargeur pour la clé Windows. |
| `bonk-hal` | Filtre de sécurité et trait `Platform` : énumération, accès brut, démontage, formatage par les outils de l'hôte, par système. Le code propre à un système d'exploitation y vit ; restent à y déplacer la détection de l'affichage (`bonk-gui`) et deux chemins Linux (`bonk-cli`, `diagnose` et `debug layout`). |
| `bonk-ops` | Porte d'écriture unique (filtre, identité, démontage, vérification) et codes d'erreur stables. Sans interface : ni clap, ni Iced. |
| `bonk-customizer` | Fichiers de réponse Windows, persistance des systèmes Live Linux. |
| `bonk-catalog` | Téléchargements reprenables, fichiers d'empreintes ; plus tard manifeste signé et bibliothèque locale. |
| `bonk-cli` | Commandes, sortie texte et JSON, codes de sortie stables. |
| `bonk-gui` | Interface graphique Iced. |

## Règles de dépendance
- `bonk-core` ne dépend d'aucun autre crate du workspace ni d'aucune API système.
- `bonk-ops` ne dépend d'aucune interface.
- `bonk-cli` et `bonk-gui` ne dépendent pas l'un de l'autre.
- Le code propre à un OS est isolé dans `bonk-hal` derrière le trait `Platform`, aux exceptions près citées plus haut.

`crates/bonk/tests/architecture.rs` vérifie les dépendances entre crates, et, pour le code propre à un OS, l'absence de `cfg(target_os`, de `libc::` et de `std::os::` hors de `bonk-hal`.

## Sécurité des disques
1. Les disques système de l'hôte (racine, démarrage, ESP active, volumes montés par le système, membres RAID/LVM, volumes chiffrés) ne sont jamais proposés ni écrits ; `bonk list --all` les affiche avec le motif de leur exclusion.
2. Par défaut, seuls les disques sur bus amovible (USB, SD, Thunderbolt externe) sont proposés ; le filtrage se fait sur le bus, pas sur le drapeau « amovible ».
3. L'identité du disque (numéro de série et taille) est revérifiée juste avant l'écriture.
4. En ligne de commande : cible désignée par un identifiant stable, `--yes` obligatoire, `--dry-run` disponible.
5. Un disque interne non système exige une option explicite et une confirmation de plus : `--force` en ligne de commande, sans question, pour rester pilotable par un script ; une fenêtre de confirmation dans l'interface. `--force` n'ouvre jamais un disque système. Un réglage ou une variable d'environnement n'autorisent jamais une écriture.

## Privilèges
L'interface ne s'exécute jamais avec des droits élevés. Seules les opérations disque le sont :

| OS | Mécanisme |
|---|---|
| Linux | UDisks2 et polkit |
| Windows | prévu : sous-processus élevé par UAC |
| macOS | prévu : `authopen` et DiskArbitration |

## Tests
- `bonk-core` est testé sur des fichiers image.
- Le filtre de sécurité est testé sur des jeux d'énumération écrits à la main d'après de vraies machines ; une capture réelle peut être rejouée (`bonk diagnose --capture`, `--fixture`).
- En CI, l'image officielle de Debian (x86_64), écrite par Bonk dans un fichier image, est démarrée dans QEMU avec OVMF, avec et sans Secure Boot, jusqu'au lancement du chargeur d'amorçage.
- Aucun test n'écrit sur un périphérique réel.
