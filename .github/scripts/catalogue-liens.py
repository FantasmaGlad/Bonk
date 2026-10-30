#!/usr/bin/env python3
"""Vérifie que chaque URL du manifeste du catalogue répond (HEAD, sans téléchargement) et écrit un rapport.

Usage : catalogue-liens.py <manifeste.json> <rapport.md>
Format attendu (J5, à confirmer) : {"entries": [{"id": "...", "url": "https://...", "sha256": "..."}]}
Code de sortie 1 si au moins un lien est cassé. Les empreintes ne sont pas recalculées ici (images de
plusieurs Go) : elles le sont à la demande, voir le workflow.
"""
import json
import sys
import urllib.error
import urllib.request


def verifier(url: str) -> str | None:
    requete = urllib.request.Request(url, method="HEAD", headers={"User-Agent": "bonk-catalogue-ci"})
    try:
        with urllib.request.urlopen(requete, timeout=30) as reponse:
            return None if reponse.status < 400 else f"HTTP {reponse.status}"
    except urllib.error.HTTPError as e:
        return f"HTTP {e.code}"
    except (urllib.error.URLError, TimeoutError) as e:
        return str(e)


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    entrees = json.load(open(sys.argv[1], encoding="utf-8"))["entries"]
    cassees = []
    for e in entrees:
        erreur = verifier(e["url"])
        if erreur:
            cassees.append((e["id"], e["url"], erreur))
    with open(sys.argv[2], "w", encoding="utf-8") as rapport:
        rapport.write(f"# Catalogue : {len(entrees)} entrées, {len(cassees)} lien(s) cassé(s)\n\n")
        for ident, url, erreur in cassees:
            rapport.write(f"- `{ident}` : {url} ({erreur})\n")
    return 1 if cassees else 0


if __name__ == "__main__":
    sys.exit(main())
