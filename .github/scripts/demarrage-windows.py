#!/usr/bin/env python3
"""Boots a Windows installation drive image under QEMU until the setup screen shows.

Windows setup writes nothing on the serial console: the proof is the screen, a solid background and the light
blue frame of the setup window, measured on Windows 10, 11 and Server 2022 boots.
"""
import argparse
import os
import shutil
import socket
import subprocess
import sys
import time

OVMF = "/usr/share/OVMF"
MONITOR = "moniteur.sock"
# Purple for the client editions, navy for Server.
SETUP_BACKGROUNDS = {(24, 0, 82), (0, 25, 64)}
WINDOW_FRAME = (185, 209, 234)
BACKGROUND_SHARE = 0.30
# The frame is about 0.5% of the screen; Windows 10 draws the window inside purple, Windows 11 in white.
FRAME_SHARE = 0.002
LOOK_SECONDS = 10


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


def read_ppm(path):
    """Width, height and RGB bytes of a binary PPM (P6), as QEMU writes it."""
    with open(path, "rb") as file:
        data = file.read()
    fields, position = [], 0
    while len(fields) < 4:
        while data[position:position + 1].isspace():
            position += 1
        if data[position:position + 1] == b"#":
            position = data.index(b"\n", position)
            continue
        end = position
        while not data[end:end + 1].isspace():
            end += 1
        fields.append(data[position:end])
        position = end
    if fields[0] != b"P6":
        raise ValueError(f"{path}: not a binary PPM")
    width, height = int(fields[1]), int(fields[2])
    return width, height, data[position + 1:position + 1 + width * height * 3]


def setup_shown(path):
    width, height, pixels = read_ppm(path)
    total = width * height
    if total == 0:
        return False, 0.0, 0.0
    background = frame = 0
    for offset in range(0, len(pixels) - 2, 3):
        pixel = (pixels[offset], pixels[offset + 1], pixels[offset + 2])
        if pixel in SETUP_BACKGROUNDS:
            background += 1
        elif pixel == WINDOW_FRAME:
            frame += 1
    background_share, frame_share = background / total, frame / total
    shown = background_share >= BACKGROUND_SHARE and frame_share >= FRAME_SHARE
    return shown, background_share, frame_share


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
        if os.path.exists(name):
            os.remove(name)
        self.sock.sendall(f"screendump {name}\n".encode())
        for _ in range(50):
            if os.path.exists(name) and os.path.getsize(name) > 0:
                time.sleep(0.2)
                return True
            time.sleep(0.1)
        return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("image")
    parser.add_argument("mode", choices=["bios", "uefi", "secureboot"])
    parser.add_argument("dossier")
    parser.add_argument("--memoire", default="4096")
    parser.add_argument("--delai", type=int, default=420)
    args = parser.parse_args()

    image = os.path.abspath(args.image)
    os.makedirs(args.dossier, exist_ok=True)
    # Relative paths: Unix socket path length is limited.
    os.chdir(args.dossier)
    if os.path.exists(MONITOR):
        os.remove(MONITOR)
    accel = ["-enable-kvm", "-cpu", "host"] if os.access("/dev/kvm", os.W_OK) else []
    print(f"{args.mode} : accélération {'KVM' if accel else 'absente'}", flush=True)
    qemu = subprocess.Popen(
        [
            "qemu-system-x86_64", *firmware(args.mode), *accel, "-m", args.memoire, "-smp", "2",
            "-display", "none", "-no-reboot", "-monitor", f"unix:{MONITOR},server,nowait",
            "-drive", f"if=none,id=cle,format=raw,snapshot=on,file={image}",
            "-device", "qemu-xhci", "-device", "usb-storage,drive=cle,bootindex=1",
        ],
        stdin=subprocess.DEVNULL,
    )
    try:
        monitor = Monitor()
        start = time.monotonic()
        name = f"{args.mode}.ppm"
        while time.monotonic() - start < args.delai and qemu.poll() is None:
            time.sleep(LOOK_SECONDS)
            if not monitor.screen(name):
                continue
            shown, background, frame = setup_shown(name)
            elapsed = int(time.monotonic() - start)
            print(f"{args.mode} {elapsed} s : fond {background:.2f}, cadre {frame:.4f}", flush=True)
            if shown:
                print(f"OK ({args.mode}) : installateur de Windows atteint en {elapsed} s")
                return 0
        print(f"ECHEC ({args.mode}) : installateur absent après {args.delai} s", file=sys.stderr)
        return 1
    finally:
        qemu.kill()
        qemu.wait()


if __name__ == "__main__":
    sys.exit(main())
