# Sécurité

Bonk écrit sur des disques entiers : une faille peut détruire des données. [English version](SECURITY.md)

Signalez une faille en privé, jamais dans une issue publique : par un [signalement privé GitHub](https://github.com/FantasmaGlad/Bonk/security/advisories/new), ou par un courriel chiffré à security@bonk-os.app. Indiquez la version, le système, les étapes et l'effet.

Récupérez la clé et vérifiez son empreinte avant de chiffrer :

```bash
gpg --locate-keys security@bonk-os.app
gpg --fingerprint security@bonk-os.app
```

`A782 AC57 321A 862C 2ED8  F0FB 34F1 5332 E2A0 7A30`

Les objectifs sont un accusé de réception sous 7 jours, une première évaluation sous 30 jours et une divulgation avec le correctif sous 90 jours. Ce sont des objectifs, pas des promesses : le projet n'a qu'un mainteneur.

La politique couvre la dernière version, les sites et les dépôts de paquets. La recherche de bonne foi est bienvenue.
