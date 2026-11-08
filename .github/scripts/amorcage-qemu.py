#!/usr/bin/env python3
"""Démarre un MBR sous QEMU (SeaBIOS) et contrôle le texte qu'il affiche.

Le texte est lu dans la mémoire de l'écran (0xB8000) par le moniteur de QEMU : aucune option du BIOS ni du
code testé n'est nécessaire. Usage : amorcage-qemu.py <mbr.bin> <texte attendu>
"""

import subprocess
import sys
import tempfile
import time
from pathlib import Path


def screen(image: Path, dump: Path) -> str:
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", "-machine", "pc", "-m", "64", "-display", "none", "-no-reboot",
         "-drive", f"file={image},format=raw,if=ide", "-monitor", "stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True,
    )
    time.sleep(6)
    qemu.communicate(f'pmemsave 0xb8000 4000 "{dump}"\nquit\n', timeout=30)
    cells = dump.read_bytes()[0::2].decode("cp437")
    return "\n".join(cells[i:i + 80].rstrip() for i in range(0, 2000, 80))


def main() -> int:
    code, expected = Path(sys.argv[1]).read_bytes(), sys.argv[2]
    disk = bytearray(1 << 20)
    disk[: len(code)] = code
    disk[510:512] = b"\x55\xaa"
    with tempfile.TemporaryDirectory() as tmp:
        image = Path(tmp, "disk.img")
        image.write_bytes(disk)
        text = screen(image, Path(tmp, "screen.bin"))
    print(text.strip())
    if expected not in text:
        print(f"ERREUR : texte attendu absent : {expected}", file=sys.stderr)
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
