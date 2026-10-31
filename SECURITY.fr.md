# Politique de sécurité

Bonk écrit sur des disques entiers : une faille peut détruire des données. Cette page explique comment signaler une vulnérabilité, comment chiffrer le signalement et ce que vous pouvez attendre en retour.

English version: [SECURITY.md](SECURITY.md). Politique lisible par machine ([RFC 9116](https://www.rfc-editor.org/rfc/rfc9116), signée OpenPGP) : <https://bonk-os.app/.well-known/security.txt>.

## Versions suivies

Les correctifs de sécurité vont à la dernière version publiée uniquement.

## Signaler une vulnérabilité

N'ouvrez ni issue ni discussion publique pour une vulnérabilité. Utilisez l'un de ces canaux privés :

1. **Signalement privé GitHub** : [signaler une vulnérabilité](https://github.com/FantasmaGlad/Bonk/security/advisories/new). Seul le mainteneur voit le signalement.
2. **Courriel** : [security@bonk-os.app](mailto:security@bonk-os.app).

Indiquez la version de Bonk, le système, les étapes pour reproduire et l'effet observé. N'y joignez ni image disque personnelle ni donnée identifiant votre matériel.

### Chiffrer le signalement

Tout courriel contenant des détails exploitables doit être chiffré avec la clé OpenPGP ci-dessous. Si vous ne pouvez pas utiliser PGP, préférez le signalement privé GitHub à un courriel en clair.

| | |
|---|---|
| Identité | Bonk Security `<security@bonk-os.app>` |
| Algorithmes | Ed25519 (certification, signature) et Curve25519 (chiffrement) |
| Empreinte | `A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30` |
| Création / expiration | 2026-10-31 / 2028-10-30 |
| Clé privée | protégée par une phrase secrète, détenue par le mainteneur, avec un certificat de révocation |

Récupérez la clé par les deux canaux et vérifiez que l'empreinte correspond avant usage :

- ce dépôt : [`docs/security/bonk-security-public-key.asc`](docs/security/bonk-security-public-key.asc)
- le site : <https://bonk-os.app/.well-known/security.asc>

```bash
curl -fsSL https://bonk-os.app/.well-known/security.asc | gpg --import
gpg --fingerprint security@bonk-os.app
# attendu : A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30
```

Pour recevoir une réponse chiffrée, joignez votre propre clé publique à votre message.

## Ce que vous pouvez attendre

| Étape | Objectif |
|---|---|
| Accusé de réception | sous 7 jours |
| Première évaluation | sous 30 jours |
| Correctif | dès que possible, annoncé dans les notes de version |
| Divulgation coordonnée | avec le correctif, au plus tard 90 jours après le signalement sauf accord contraire |

Ce sont des objectifs, pas des engagements contractuels : le projet n'a qu'un mainteneur. Les auteurs de signalement qui le souhaitent sont crédités.

## Périmètre

Concernés : le code de ce dépôt, ses binaires et installateurs publiés, ses workflows, le site `bonk-os.app` et les dépôts de paquets `packages.bonk-os.app`.

Non concernés : les failles de services tiers (GitHub, Cloudflare, Resend), l'ingénierie sociale, le déni de service et les analyses automatisées de masse, les constats sans impact démontrable. Une vulnérabilité d'une dépendance se signale d'abord à son projet ; prévenez-nous si elle touche Bonk.

## Recherche de bonne foi

Une recherche menée de bonne foi dans ce périmètre n'entraînera aucune action de notre part. Essayez sur vos propres disques et images ; n'accédez pas aux données d'autrui et ne perturbez pas les services.

## Vérifier ce que nous publions

Chaque version porte un fichier `SHA256SUMS` et des attestations de construction :

```bash
gh attestation verify <fichier> --repo FantasmaGlad/Bonk
```

Les dépôts de paquets sont signés par une clé dédiée, distincte de la clé de signalement ci-dessus et jamais utilisée pour recevoir des signalements :

```bash
curl -fsSL https://packages.bonk-os.app/bonk.asc | gpg --show-keys --fingerprint
# attendu : 8DE6 80FF BE47 5B66 3E8B  019A 7578 66E3 4D67 014B
```

## Historique du document

| Date | Changement |
|---|---|
| 2026-10-31 | Clé de signalement `A782 AC57 321A 862C 2ED8 F0FB 34F1 5332 E2A0 7A30` créée et publiée. `security.txt` signé avec cette clé. |

Cet historique liste tout changement de clé, d'empreinte ou d'engagement.
