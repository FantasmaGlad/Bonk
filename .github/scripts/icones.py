#!/usr/bin/env python3
"""Régénère toutes les icônes depuis le logo : changer de logo, c'est remplacer `bonk-logo.png` et relancer ceci.

Usage, depuis le dépôt : python3 .github/scripts/icones.py
Sorties : .ico pour Windows, .icns pour macOS, PNG par taille pour les bureaux Linux, RGBA brut pour la fenêtre.
"""
from pathlib import Path

from PIL import Image

ACTIFS = Path("crates/bonk/assets")
TAILLES_LINUX = (16, 24, 32, 48, 64, 128, 256, 512)
TAILLES_WINDOWS = (16, 24, 32, 48, 64, 128, 256)
FENETRE = 64

logo = Image.open(ACTIFS / "bonk-logo.png").convert("RGBA")
assert logo.width == logo.height, "le logo doit être carré"
reduit = lambda taille: logo.resize((taille, taille), Image.Resampling.LANCZOS)

for taille in TAILLES_LINUX:
    dossier = ACTIFS / "icons" / f"{taille}x{taille}"
    dossier.mkdir(parents=True, exist_ok=True)
    reduit(taille).save(dossier / "bonk.png", optimize=True)
logo.save(ACTIFS / "bonk.ico", sizes=[(t, t) for t in TAILLES_WINDOWS])
logo.save(ACTIFS / "bonk.icns")
Path("crates/bonk-gui/assets").mkdir(exist_ok=True)
Path("crates/bonk-gui/assets/bonk-64.rgba").write_bytes(reduit(FENETRE).tobytes())
