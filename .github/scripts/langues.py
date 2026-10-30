#!/usr/bin/env python3
"""Vérifie que les catalogues de langues de l'interface ont les mêmes clés (format Fluent, un fichier .ftl
par langue et par domaine : <dossier>/<langue>/<domaine>.ftl). Une clé manquante ou en trop échoue.

Usage : langues.py <dossier des catalogues> [langue de référence, défaut fr]
"""
import re
import sys
from pathlib import Path

CLE = re.compile(r"^([A-Za-z][A-Za-z0-9_-]*)\s*=", re.MULTILINE)


def cles(dossier: Path) -> dict[str, set[str]]:
    """domaine -> clés."""
    return {f.stem: set(CLE.findall(f.read_text(encoding="utf-8"))) for f in sorted(dossier.glob("*.ftl"))}


def main() -> int:
    if len(sys.argv) not in (2, 3):
        print(__doc__)
        return 2
    racine, reference = Path(sys.argv[1]), (sys.argv[2] if len(sys.argv) == 3 else "fr")
    langues = sorted(d.name for d in racine.iterdir() if d.is_dir())
    if reference not in langues:
        print(f"langue de référence absente : {reference}")
        return 1
    base = cles(racine / reference)
    erreurs = 0
    for langue in langues:
        if langue == reference:
            continue
        autre = cles(racine / langue)
        for domaine in sorted(set(base) | set(autre)):
            manquantes = base.get(domaine, set()) - autre.get(domaine, set())
            en_trop = autre.get(domaine, set()) - base.get(domaine, set())
            for cle in sorted(manquantes):
                print(f"{langue}/{domaine}.ftl : clé manquante {cle}")
            for cle in sorted(en_trop):
                print(f"{langue}/{domaine}.ftl : clé absente de {reference} : {cle}")
            erreurs += len(manquantes) + len(en_trop)
    print(f"{len(langues)} langue(s), {erreurs} écart(s)")
    return 1 if erreurs else 0


if __name__ == "__main__":
    sys.exit(main())
