# Sécurité

Bonk écrit sur des disques entiers : une faille peut détruire des données.

[English version](SECURITY.md)

## Signaler

En privé, jamais dans une issue publique :

- [Signalement privé GitHub](https://github.com/FantasmaGlad/Bonk/security/advisories/new)
- security@bonk-os.app, chiffré

Indiquez la version, le système, les étapes et l'effet.

## Chiffrer

```bash
gpg --locate-keys security@bonk-os.app
gpg --fingerprint security@bonk-os.app
```

Empreinte attendue :

`A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30`

## Ce que vous pouvez attendre

- Accusé de réception sous 7 jours
- Première évaluation sous 30 jours
- Divulgation avec le correctif, sous 90 jours

Ce sont des objectifs : le projet n'a qu'un mainteneur.

## Périmètre

La dernière version, les sites et les dépôts de paquets. La recherche de bonne foi est bienvenue.
