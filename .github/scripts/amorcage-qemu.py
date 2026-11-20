#!/usr/bin/env python3
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MBR = Path("crates/bonk-core/src/boot/mbr.bin")
VBR = Path("crates/bonk-core/src/boot/vbr.bin")
STAGE2 = Path("crates/bonk-core/src/boot/stage2.bin")
TEST_VBR = Path("boot/test-vbr.bin")
TEST_BOOTMGR = Path("boot/test-bootmgr.bin")
START = 2048
RESERVED = 32
END = 0x0FFFFFFF


def disk(active, vbr, flags=None, first=None):
    """Partitions 1 and 3 hold `vbr`; `flags` overrides the boot indicators, `first` the start of partition 1."""
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
        if flags is not None:
            image[entry] = flags[index]
    if first is not None:
        image[454:458] = first.to_bytes(4, "little")
    for index in (0, 2):
        at = START * (index + 1) * 512
        image[at : at + 512] = vbr
    image[510:512] = b"\x55\xaa"
    return image


def fake_bootmgr(size):
    data = bytearray(size)
    head = TEST_BOOTMGR.read_bytes()[: min(size, 504)]
    data[: len(head)] = head
    data[4:8] = size.to_bytes(4, "little")
    for block in range(1, size // 4096 + 1):
        if block * 4096 + 2 <= size - 8:
            data[block * 4096 : block * 4096 + 2] = block.to_bytes(2, "little")
    data[size - 8 :] = b"BONKEND!"
    return bytes(data)


def entry(name, attribute, cluster=0, size=0):
    raw = bytearray(32)
    raw[:11] = name.ljust(11).encode()
    raw[11] = attribute
    raw[20:22] = (cluster >> 16).to_bytes(2, "little")
    raw[26:28] = (cluster & 0xFFFF).to_bytes(2, "little")
    raw[28:32] = size.to_bytes(4, "little")
    return bytes(raw)


def fat32(spc, payload=None, fragmented=False, truncated=False, **damage):
    """MBR disk with an active FAT32 partition; `payload` is BOOTMGR's content, None when missing.

    `damage`: `start` and `size` override BOOTMGR's entry, `loop` points its first cluster to itself, `bad`
    marks the link after its first cluster as a bad cluster, `root_next` replaces the link after the root
    directory's first cluster, which is full, `bpb` overrides boot sector fields, `active_fat` selects the FAT
    the boot sector names and spoils the other one, `no_stage2` leaves the second stage out."""
    shape = layout(spc, payload, fragmented)
    table, root, chain = shape["table"], shape["root"], shape["chain"]
    if truncated and len(chain) > 1:
        table[chain[len(chain) // 2]] = END
    if damage.get("loop"):
        table[chain[0]] = chain[0]
    if damage.get("bad"):
        table[chain[0]] = 0x0FFFFFF7
    if "root_next" in damage:
        table[root[0]] = damage["root_next"]

    volume = bytearray(shape["total"] * 512)
    sector = boot_sector(spc, shape, damage)
    volume[:512] = sector
    volume[6 * 512 : 7 * 512] = sector
    if not damage.get("no_stage2"):
        volume[9 * 512 : 9 * 512 + 4096] = STAGE2.read_bytes()
    write_fats(volume, table, shape["fat_sectors"], damage.get("active_fat"))
    directory = root_directory(payload, chain, spc, damage)
    write_chain(volume, root, directory, shape["data_lba"], spc)
    write_chain(volume, chain, payload or b"", shape["data_lba"], spc)
    return with_mbr(volume, shape["total"])


def layout(spc, payload, fragmented):
    """Cluster map: a full first root cluster, then BOOTMGR's chain, in order or reversed every other one."""
    cluster_bytes = spc * 512
    file_clusters = -(-len(payload) // cluster_bytes) if payload else 0
    per_cluster = cluster_bytes // 32
    root_clusters = -(-(per_cluster + 5) // per_cluster)
    clusters = max(64, 2 * (file_clusters + root_clusters) + 16)
    fat_sectors = -(-(clusters + 2) * 4 // 512)
    data_lba = RESERVED + 2 * fat_sectors
    table = [0x0FFFFFF8, END] + [0] * clusters
    root = list(range(2, 2 + root_clusters))
    free = list(range(2 + root_clusters, 2 + clusters))
    chain = free[: 2 * file_clusters : 2][::-1] if fragmented else free[:file_clusters]
    for run in (root, chain):
        for here, after in zip(run, run[1:] + [END]):
            table[here] = after
    return {"table": table, "root": root, "chain": chain, "fat_sectors": fat_sectors,
            "data_lba": data_lba, "total": data_lba + clusters * spc}


def boot_sector(spc, shape, damage):
    sector = bytearray(VBR.read_bytes())
    bpb = {0x0B: (512, 2), 0x0D: (spc, 1), 0x0E: (RESERVED, 2), 0x10: (2, 1), 0x15: (0xF8, 1),
           0x18: (63, 2), 0x1A: (255, 2), 0x1C: (START, 4), 0x20: (shape["total"], 4),
           0x24: (shape["fat_sectors"], 4), 0x2C: (2, 4), 0x30: (1, 2), 0x32: (6, 2), 0x40: (0x80, 1),
           0x42: (0x29, 1)}
    if damage.get("active_fat") is not None:
        bpb[0x28] = (0x80 | damage["active_fat"], 2)
    bpb.update(damage.get("bpb", {}))
    for offset, (value, width) in bpb.items():
        sector[offset : offset + width] = value.to_bytes(width, "little")
    sector[0x47:0x5A] = b"BONK       FAT32   "
    return bytes(sector)


def write_fats(volume, table, fat_sectors, active_fat):
    raw = b"".join(value.to_bytes(4, "little") for value in table)
    for copy in range(2):
        at = (RESERVED + copy * fat_sectors) * 512
        spoiled = active_fat is not None and copy != active_fat
        volume[at : at + len(raw)] = bytes(len(raw)) if spoiled else raw


def root_directory(payload, chain, spc, damage):
    """Label and decoys fill the first cluster; a BOOTMGR directory precedes the BOOTMGR file."""
    decoys = spc * 16 + 2
    entries = [entry("BONK", 0x08)] + [entry(f"FILE{n:04}TXT", 0x20, 0, 0) for n in range(decoys)]
    entries.append(entry("BOOTMGR", 0x10))
    if payload is not None:
        first = damage.get("start", chain[0] if chain else 0)
        entries.append(entry("BOOTMGR", 0x21, first, damage.get("size", len(payload))))
    return b"".join(entries)


def write_chain(volume, clusters, data, data_lba, spc):
    cluster_bytes = spc * 512
    for index, cluster in enumerate(clusters):
        at = (data_lba + (cluster - 2) * spc) * 512
        piece = data[index * cluster_bytes : (index + 1) * cluster_bytes]
        volume[at : at + len(piece)] = piece


def with_mbr(volume, total):
    """The volume behind an MBR whose single active FAT32 LBA partition starts at START."""
    image = bytearray(START * 512) + volume
    image[:440] = MBR.read_bytes()
    image[440:444] = b"BONK"
    image[446] = 0x80
    image[450] = 0x0C
    image[454:458] = START.to_bytes(4, "little")
    image[458:462] = total.to_bytes(4, "little")
    image[510:512] = b"\x55\xaa"
    return image


def screen(image, tmp, failing=()):
    """Text screen after boot; each sector of `failing` fails its first read, as a flaky USB drive does."""
    path, dump = Path(tmp, "disk.img"), Path(tmp, "screen.bin")
    path.write_bytes(image)
    drive = str(path)
    if failing:
        rules = Path(tmp, "errors.conf")
        rules.write_text("".join(
            f'[inject-error]\nevent = "read_aio"\nerrno = "5"\nsector = "{sector}"\nonce = "on"\n'
            for sector in failing))
        drive = f"blkdebug:{rules}:{path}"
    qemu = subprocess.Popen(
        ["qemu-system-x86_64", "-machine", "pc", "-m", "64", "-display", "none", "-no-reboot",
         "-drive", f"file={drive},format=raw,if=ide", "-monitor", "stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, text=True,
    )
    time.sleep(6)
    qemu.communicate(f'pmemsave 0xb8000 4000 "{dump}"\nquit\n', timeout=30)
    cells = dump.read_bytes()[0::2].decode("cp437")
    return "\n".join(cells[i : i + 80].rstrip() for i in range(0, 2000, 80))


def written_by_bonk(bonk, tmp):
    """Key written by Bonk from a fake Windows ISO: x86_64 UEFI loader, WIM and fake BOOTMGR."""
    tree = Path(tmp, "iso")
    (tree / "efi" / "boot").mkdir(parents=True)
    (tree / "sources").mkdir()
    loader = bytearray(1024)
    loader[:2] = b"MZ"
    loader[0x3C] = 0x40
    loader[0x40:0x44] = b"PE\0\0"
    loader[0x44:0x46] = (0x8664).to_bytes(2, "little")
    loader[0x58:0x5A] = (0x20B).to_bytes(2, "little")
    (tree / "efi" / "boot" / "bootx64.efi").write_bytes(loader)
    (tree / "sources" / "install.wim").write_bytes(bytes(range(256)) * 800)
    (tree / "bootmgr").write_bytes(fake_bootmgr(473_364))
    iso, key = Path(tmp, "win.iso"), Path(tmp, "cle.img")
    subprocess.run(["genisoimage", "-quiet", "-udf", "-R", "-J", "-o", str(iso), str(tree)], check=True)
    subprocess.run(
        [bonk, "write", "--simulate", str(key), str(iso), "--size", str(256 << 20),
         "--table", "mbr", "--yes", "--progress", "none"],
        check=True, stdout=subprocess.DEVNULL,
    )
    shutil.rmtree(tree)
    return key.read_bytes()


def cases(vbr):
    unsigned = vbr[:510] + b"\0\0"
    return [
        ("partition 1 active", disk(0, vbr), "BONK: handover OK"),
        ("partition 3 active", disk(2, vbr), "BONK: handover OK"),
        ("no active partition", disk(None, vbr), "Bonk: no active partition"),
        ("boot sector without signature", disk(0, unsigned), "Bonk: no boot sector"),
        ("deux partitions actives", disk(0, vbr, flags=(0x80, 0, 0x80, 0)), "Bonk: invalid partition table"),
        ("indicateur d'amorçage invalide", disk(0, vbr, flags=(0x80, 0x01, 0, 0)), "Bonk: invalid partition table"),
        ("partition active au secteur 0", disk(0, vbr, first=0), "Bonk: invalid partition table"),
    ] + fat32_cases()


def fat32_cases():
    return [
        ("FAT32, BOOTMGR d'un secteur", fat32(1, fake_bootmgr(512)), "BONK: bootmgr OK"),
        ("FAT32, BOOTMGR au-delà de 64 Kio", fat32(1, fake_bootmgr(70_000)), "BONK: bootmgr OK"),
        ("FAT32, BOOTMGR de 470 Kio", fat32(8, fake_bootmgr(481_280)), "BONK: bootmgr OK"),
        ("FAT32, clusters de 4 Kio, chaîne fragmentée", fat32(8, fake_bootmgr(200_000), fragmented=True), "BONK: bootmgr OK"),
        ("FAT32, clusters de 32 Kio", fat32(64, fake_bootmgr(473_364)), "BONK: bootmgr OK"),
        ("FAT32, clusters de 64 Kio", fat32(128, fake_bootmgr(473_364)), "BONK: bootmgr OK"),
        ("FAT32, seconde FAT active, première abîmée", fat32(8, fake_bootmgr(200_000), fragmented=True, active_fat=1), "BONK: bootmgr OK"),
        ("FAT32, BOOTMGR trop grand", fat32(8, fake_bootmgr(600_000)), "Bonk: BOOTMGR too large"),
        ("FAT32, taille de 4 Gio sur une chaîne en boucle", fat32(8, fake_bootmgr(70_000), loop=True, size=0xFFFFFF00), "Bonk: BOOTMGR too large"),
        ("FAT32, BOOTMGR absent", fat32(8, None), "Bonk: BOOTMGR missing"),
        ("FAT32, BOOTMGR vide", fat32(8, b""), "Bonk: BOOTMGR missing"),
        ("FAT32, chaîne tronquée", fat32(8, fake_bootmgr(200_000), truncated=True), "Bonk: BOOTMGR damaged"),
        ("FAT32, BOOTMGR au cluster 0", fat32(8, fake_bootmgr(70_000), start=0), "Bonk: BOOTMGR damaged"),
        ("FAT32, BOOTMGR hors du volume", fat32(8, fake_bootmgr(70_000), start=0x0FFFFF00), "Bonk: BOOTMGR damaged"),
        ("FAT32, cluster défectueux dans la chaîne", fat32(8, fake_bootmgr(200_000), bad=True), "Bonk: BOOTMGR damaged"),
        ("FAT32, répertoire chaîné vers un cluster libre", fat32(1, None, root_next=0), "Bonk: volume damaged"),
        ("FAT32, second niveau absent", fat32(8, fake_bootmgr(70_000), no_stage2=True), "Bonk: second stage missing"),
        ("FAT32, secteurs de 4096 octets annoncés", fat32(8, fake_bootmgr(70_000), bpb={0x0B: (4096, 2)}), "Bonk: unsupported volume"),
        ("FAT32, 3 secteurs par cluster", fat32(8, fake_bootmgr(70_000), bpb={0x0D: (3, 1)}), "Bonk: unsupported volume"),
    ]


def flaky_cases():
    image = fat32(8, fake_bootmgr(473_364), fragmented=True)
    return [
        ("lecture ratée une fois : secteur d'amorçage", (image, (START,)), "BONK: bootmgr OK"),
        ("lecture ratée une fois : second niveau", (image, (START + 9,)), "BONK: bootmgr OK"),
        ("lecture ratée une fois : FAT et BOOTMGR", (image, (START + RESERVED, START + 4000)), "BONK: bootmgr OK"),
    ]


def main():
    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        checks = cases(TEST_VBR.read_bytes()) + flaky_cases()
        if len(sys.argv) > 1:
            checks.append(("clé écrite par bonk write --table mbr", written_by_bonk(sys.argv[1], tmp), "BONK: bootmgr OK"))
        for name, image, expected in checks:
            image, failing = image if isinstance(image, tuple) else (image, ())
            text = screen(image, tmp, failing)
            ok = expected in text
            failed += not ok
            print(f"{'OK' if ok else 'ERREUR'} {name}: {expected}")
            if not ok:
                print(text.strip())
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
