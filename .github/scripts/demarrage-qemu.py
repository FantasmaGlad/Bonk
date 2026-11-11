#!/usr/bin/env python3
import argparse
import os
import platform
import re
import shutil
import socket
import subprocess
import sys
import time

OVMF = "/usr/share/OVMF"
AAVMF = "/usr/share/AAVMF"
# The BIOS loader only writes to the screen: fixed wait, shorter than the menu countdown.
BIOS_MENU_SECONDS = 25
# The GRUB menu shows shortly after the firmware banner.
UEFI_BOOT_LIMIT_SECONDS = 180
GRUB_MENU_SECONDS = 8
KEY_GAP_SECONDS = 0.15
CONSOLE_KEYS = "spc i g n o r e shift-minus l o g l e v e l spc c o n s o l e equal t t y shift-s 0".split()
MONITOR = "moniteur.sock"
SERIAL = "serie.log"


def firmware(mode, processor):
    if processor == "aarch64":
        if mode != "uefi":
            raise SystemExit("aarch64 : UEFI seulement")
        shutil.copy(os.path.join(AAVMF, "AAVMF_VARS.fd"), "vars.fd")
        # No network: `virt` would add a virtio card whose boot ROM is not installed.
        return [
            "-machine", "virt", "-nic", "none",
            "-drive", f"if=pflash,format=raw,readonly=on,file={AAVMF}/AAVMF_CODE.no-secboot.fd",
            "-drive", "if=pflash,format=raw,file=vars.fd",
        ]
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


# Printed only when a boot has failed for good: waiting the whole delay would prove nothing more.
DEAD_ENDS = re.compile(
    r"Kernel panic - not syncing"
    r"|\(initramfs\) "
    r"|Could not find the ISO"
    r"|Unable to find a medium containing a live file system"
    r"|No BonkVault partition found"
    r"|You are in emergency mode"
    r"|ERROR: .* device did not show up"
)


def wait_for(pattern, limit, qemu):
    end = time.monotonic() + limit
    while time.monotonic() < end and qemu.poll() is None:
        text = serial_text()
        if re.search(pattern, text):
            return True
        dead_end = DEAD_ENDS.search(text)
        if dead_end:
            print(f"impasse sur la console série : {dead_end.group(0)}", file=sys.stderr)
            return False
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
    """Official images are silent on the serial console: add `console=ttyS0` to the kernel line, as a person would in the boot menu."""
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
    parser.add_argument("--processeur", choices=["x86_64", "aarch64"], default="x86_64")
    args = parser.parse_args()

    image = os.path.abspath(args.image)
    os.makedirs(args.dossier, exist_ok=True)
    # Relative paths: Unix socket path length is limited.
    os.chdir(args.dossier)
    for old in (SERIAL, MONITOR):
        if os.path.exists(old):
            os.remove(old)
    x86 = args.processeur == "x86_64"
    # KVM only runs the runner's own processor; emulated, `max` is too slow to reach the menu in minutes.
    kvm = platform.machine() == args.processeur and os.access("/dev/kvm", os.W_OK)
    accel = ["-enable-kvm", "-cpu", "host"] if kvm else ([] if x86 else ["-cpu", "cortex-a72"])
    print(f"{args.mode} {args.processeur} : accélération {'KVM' if kvm else 'absente'}", flush=True)
    qemu = subprocess.Popen(
        [
            f"qemu-system-{args.processeur}", *firmware(args.mode, args.processeur), *accel,
            "-m", args.memoire, "-smp", "2",
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
        # ARM64 firmware names its serial port to the kernel (SPCR): nothing to add, and no screen to dump.
        if x86:
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
