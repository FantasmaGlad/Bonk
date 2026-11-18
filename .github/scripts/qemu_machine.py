import os
import shutil
import socket
import time

OVMF = "/usr/share/OVMF"
AAVMF = "/usr/share/AAVMF"


def firmware(mode, processor="x86_64"):
    """QEMU machine and firmware arguments; the writable variables are copied to vars.fd."""
    if processor == "aarch64":
        if mode != "uefi":
            raise SystemExit("aarch64 : UEFI seulement")
        shutil.copy(os.path.join(AAVMF, "AAVMF_VARS.fd"), "vars.fd")
        # Emulated: `max` adds features whose emulation is too slow to reach the menu in minutes.
        # No network: `virt` would add a virtio card whose boot ROM is not installed.
        return [
            "-machine", "virt", "-cpu", "cortex-a72", "-nic", "none",
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


def connect_monitor(path):
    """The QEMU monitor socket, waited for while QEMU starts."""
    sock = socket.socket(socket.AF_UNIX)
    for _ in range(100):
        try:
            sock.connect(path)
            return sock
        except OSError:
            time.sleep(0.1)
    raise SystemExit("moniteur QEMU injoignable")
