#!/usr/bin/env python3
"""Démarre le MBR de Bonk sous QEMU (SeaBIOS) dans plusieurs dispositions de disque.

Le texte est lu dans la mémoire de l'écran (0xB8000) par le moniteur de QEMU. Usage, depuis le dépôt :
python3 .github/scripts/amorcage-qemu.py
"""

import subprocess
import sys
import tempfile
import time
from pathlib import Path

MBR = Path("crates/bonk-core/src/boot/mbr.bin")
TEST_VBR = Path("boot/test-vbr.bin")
START = 2048


def disk(active, vbr):
    image = bytearray(4 << 20)
    image[:440] = MBR.read_bytes()
    image[440:444] = b"BONK"
    for index in range(4):
        entry = 446 + 16 * index
        if index in (0, 2):
            image[entry] = 0x80 if index == active else 0
            image[entry + 4] = 0x0C
            image[entry + 8 : entry + 12] = (START * (index + 1)).to_bytes(4, "little")
            image[entry + 12 : entry + 16] = (2048).to_bytes(4, "little")
    for index in (0, 2):
        at = START * (index + 1) * 512
        image[at : at + 512] = vbr
    image[510:512] = b"\x55\xaa"
    return image


def screen(image, tmp):
    path, dump = Path(tmp, "disk.img"), Path(tmp, "screen.bin")
    path.write_bytes(image)
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", "-machine", "pc", "-m", "64", "-display", "none", "-no-reboot",
         "-drive", f"file={path},format=raw,if=ide", "-monitor", "stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True,
    )
    time.sleep(6)
    qemu.communicate(f'pmemsave 0xb8000 4000 "{dump}"\nquit\n', timeout=30)
    cells = dump.read_bytes()[0::2].decode("cp437")
    return "\n".join(cells[i : i + 80].rstrip() for i in range(0, 2000, 80))


def main():
    vbr = TEST_VBR.read_bytes()
    unsigned = vbr[:510] + b"\0\0"
    cases = [
        ("partition 1 active", disk(0, vbr), "BONK: handover OK"),
        ("partition 3 active", disk(2, vbr), "BONK: handover OK"),
        ("no active partition", disk(None, vbr), "Bonk: no active partition"),
        ("boot sector without signature", disk(0, unsigned), "Bonk: no boot sector"),
    ]
    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, image, expected in cases:
            text = screen(image, tmp)
            ok = expected in text
            failed += not ok
            print(f"{'OK' if ok else 'ERREUR'} {name}: {expected}")
            if not ok:
                print(text.strip())
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
