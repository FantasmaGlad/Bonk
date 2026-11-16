#!/usr/bin/env python3
"""Fail when a source file exceeds its line limit, outside the justified exceptions."""
import sys
from pathlib import Path

PRODUCTION = 400
TESTS = 600

# An exception no longer needed also fails, so the list can only shrink.
EXCEPTIONS = {
    "crates/bonk-ops/src/errors.rs": "table statique des codes d'erreur, lue d'un seul tenant",
}


def est_un_test(chemin: Path) -> bool:
    parties = chemin.parts
    return (
        chemin.name == "tests.rs"
        or "tests" in parties
        or any(partie.endswith("_tests") for partie in parties)
    )


def lignes_de_production(texte: str) -> int:
    lignes = texte.split("\n")
    for numero, ligne in enumerate(lignes):
        if ligne == "#[cfg(test)]":
            return numero
    return len(lignes) - 1


def mesurer() -> dict[str, tuple[int, int]]:
    mesures = {}
    sources = list(Path("crates").glob("*/src/**/*.rs")) + list(Path("crates").glob("*/tests/**/*.rs"))
    sources += list(Path("android/app/src").glob("**/*.kt"))
    for chemin in sorted(sources):
        texte = chemin.read_text(encoding="utf-8")
        if est_un_test(chemin):
            mesures[chemin.as_posix()] = (len(texte.split("\n")) - 1, TESTS)
        else:
            mesures[chemin.as_posix()] = (lignes_de_production(texte), PRODUCTION)
    return mesures


def main() -> int:
    mesures = mesurer()
    erreurs = []
    for chemin, (lignes, limite) in mesures.items():
        if lignes > limite and chemin not in EXCEPTIONS:
            erreurs.append(f"{chemin} : {lignes} lignes, limite {limite} ; découper le fichier")
    for chemin in EXCEPTIONS:
        lignes, limite = mesures.get(chemin, (0, PRODUCTION))
        if lignes <= limite:
            erreurs.append(f"{chemin} : exception devenue inutile ({lignes} lignes), la retirer")
    for erreur in erreurs:
        print(erreur, file=sys.stderr)
    return 1 if erreurs else 0


if __name__ == "__main__":
    sys.exit(main())
