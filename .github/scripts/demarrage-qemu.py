#!/usr/bin/env python3
"""Démarre un fichier image sous QEMU et attend, sur la console série, la preuve que le système est atteint.

Les images officielles ne parlent pas sur la console série : le clavier virtuel ajoute `console=ttyS0` à la
ligne du noyau, comme le ferait une personne dans le menu de démarrage. L'image n'est pas modifiée.

Usage : demarrage-qemu.py <image> <bios|uefi|secureboot> <grub|isolinux|systemd-boot> <preuve> <dossier>
        [--entree N] [--ligne N] [--memoire Mio] [--delai s] [--attente s]
Code 0 : preuve vue. 1 : absente à la fin du délai. La trace reste dans <dossier>/serie.log, l'écran dans
<dossier>/*.png.
"""
import argparse
import os
import re
import shutil
import socket
import subprocess
import sys
import time

OVMF = "/usr/share/OVMF"
# Le chargeur BIOS ne parle que sur l'écran : attente fixe, plus courte que le compte à rebours du menu quand
# il en a un (`--attente`).
BIOS_MENU_SECONDS = 25
# Le menu GRUB s'affiche peu après l'annonce du micrologiciel.
UEFI_BOOT_LIMIT_SECONDS = 180
GRUB_MENU_SECONDS = 8
KEY_GAP_SECONDS = 0.15
CONSOLE_KEYS = "spc i g n o r e shift-minus l o g l e v e l spc c o n s o l e equal t t y shift-s 0".split()
MONITOR = "moniteur.sock"
SERIAL = "serie.log"


def firmware(mode):
    if mode == "bios":
        return ["-machine", "q35"]
    code, variables, machine, extra = {
        "uefi": ("OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd", "q35", []),
        "secureboot": (
            "OVMF_CODE_4M.secboot.fd",
            "OVMF_VARS_4M.ms.fd",
            "q35,smm=on",
            ["-global", "driver=cfi.pflash01,property=secure,value=on"],
        ),
    }[mode]
    shutil.copy(os.path.join(OVMF, variables), "vars.fd")
    return [
        "-machine", machine, *extra,
        "-drive", f"if=pflash,format=raw,readonly=on,file={OVMF}/{code}",
        "-drive", "if=pflash,format=raw,file=vars.fd",
    ]


def serial_text():
    try:
        with open(SERIAL, "rb") as log:
            return log.read().decode("utf-8", "replace")
    except FileNotFoundError:
        return ""


def wait_for(pattern, limit, qemu):
    end = time.monotonic() + limit
    while time.monotonic() < end and qemu.poll() is None:
        if re.search(pattern, serial_text()):
            return True
        time.sleep(0.5)
    return bool(re.search(pattern, serial_text()))


class Monitor:
    def __init__(self):
        self.sock = socket.socket(socket.AF_UNIX)
        for _ in range(100):
            try:
                self.sock.connect(MONITOR)
                return
            except OSError:
                time.sleep(0.1)
        raise SystemExit("moniteur QEMU injoignable")

    def screen(self, name):
        self.sock.sendall(f"screendump {name} -f png\n".encode())
        time.sleep(0.5)

    def keys(self, *names):
        for name in names:
            self.sock.sendall(f"sendkey {name}\n".encode())
            time.sleep(KEY_GAP_SECONDS)


def add_console(monitor, loader, entry, line):
    """Choisit l'entrée du menu et ajoute la console série à la ligne du noyau."""
    monitor.keys(*["down"] * entry)
    if loader == "grub":
        monitor.keys("e")
        time.sleep(1)
        monitor.keys(*["down"] * line, "end", *CONSOLE_KEYS)
        monitor.screen("edition.png")
        monitor.keys("ctrl-x")
    else:
        monitor.keys("e" if loader == "systemd-boot" else "tab")
        time.sleep(1)
        if loader == "systemd-boot":
            monitor.keys("end")
        monitor.keys(*CONSOLE_KEYS)
        monitor.screen("edition.png")
        monitor.keys("ret")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("image")
    parser.add_argument("mode", choices=["bios", "uefi", "secureboot"])
    parser.add_argument("chargeur", choices=["grub", "isolinux", "systemd-boot"])
    parser.add_argument("preuve")
    parser.add_argument("dossier")
    parser.add_argument("--entree", type=int, default=0, help="rang de l'entrée du menu")
    parser.add_argument("--ligne", type=int, default=3, help="rang de la ligne du noyau dans l'éditeur de GRUB")
    parser.add_argument("--memoire", default="2048")
    parser.add_argument("--delai", type=int, default=600)
    parser.add_argument("--attente", type=int, help="secondes avant le menu, en remplacement du défaut")
    args = parser.parse_args()

    image = os.path.abspath(args.image)
    os.makedirs(args.dossier, exist_ok=True)
    # Chemins relatifs : la longueur d'un socket Unix est limitée.
    os.chdir(args.dossier)
    for old in (SERIAL, MONITOR):
        if os.path.exists(old):
            os.remove(old)
    accel = ["-enable-kvm", "-cpu", "host"] if os.access("/dev/kvm", os.W_OK) else []
    print(f"{args.mode} : accélération {'KVM' if accel else 'absente'}", flush=True)
    qemu = subprocess.Popen(
        [
            "qemu-system-x86_64", *firmware(args.mode), *accel, "-m", args.memoire, "-smp", "2",
            "-display", "none", "-no-reboot", "-serial", f"file:{SERIAL}",
            "-monitor", f"unix:{MONITOR},server,nowait",
            "-drive", f"if=none,id=cle,format=raw,readonly=on,file={image}",
            "-device", "qemu-xhci", "-device", "usb-storage,drive=cle,bootindex=1",
        ],
        stdin=subprocess.DEVNULL,
    )
    try:
        monitor = Monitor()
        if args.mode == "bios":
            time.sleep(args.attente if args.attente is not None else BIOS_MENU_SECONDS)
        else:
            wait_for(r"BdsDxe: starting", UEFI_BOOT_LIMIT_SECONDS, qemu)
            time.sleep(args.attente if args.attente is not None else GRUB_MENU_SECONDS)
        monitor.screen("menu.png")
        add_console(monitor, args.chargeur, args.entree, args.ligne)
        if wait_for(args.preuve, args.delai, qemu):
            monitor.screen("atteint.png")
            print(f"OK ({args.mode}) : vu sur la console série : {args.preuve}")
            return 0
        monitor.screen("echec.png")
        print(f"ECHEC ({args.mode}) : preuve absente : {args.preuve}", file=sys.stderr)
        print(serial_text()[-2000:], file=sys.stderr)
        return 1
    finally:
        qemu.kill()
        qemu.wait()


if __name__ == "__main__":
    sys.exit(main())
