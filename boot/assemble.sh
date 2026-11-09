#!/bin/sh
set -eu
cd "$(dirname "$0")"
sysroot=$(rustc --print sysroot)
host=$(rustc -vV | sed -n 's/^host: //p')
bin="$sysroot/lib/rustlib/$host/bin"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
rustc --edition 2024 --crate-type lib --emit obj --target x86_64-unknown-linux-gnu asm.rs -o "$out/boot.o"

# Section, run address, output file.
build() {
    printf 'SECTIONS { %s %s : { KEEP(*(%s)) } /DISCARD/ : { *(.bonk_*) } }\n' "$1" "$2" "$1" > "$out/link.ld"
    LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-lld" -flavor gnu -m elf_x86_64 -e 0 -T "$out/link.ld" \
        -o "$out/link.elf" "$out/boot.o"
    LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-objcopy" -O binary -j "$1" "$out/link.elf" "$3"
    chmod 644 "$3"
}
build .bonk_mbr 0x0600 ../crates/bonk-core/src/boot/mbr.bin
build .bonk_vbr 0x7c00 ../crates/bonk-core/src/boot/vbr.bin
build .bonk_stage2 0x7e00 ../crates/bonk-core/src/boot/stage2.bin
build .bonk_test_vbr 0x7c00 test-vbr.bin
build .bonk_test_bootmgr 0 test-bootmgr.bin
