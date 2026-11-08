#!/bin/sh
# Assemble le code d'amorçage avec la seule chaîne Rust ; la CI relance ce script et refuse tout écart.
set -eu
cd "$(dirname "$0")"
sysroot=$(rustc --print sysroot)
host=$(rustc -vV | sed -n 's/^host: //p')
bin="$sysroot/lib/rustlib/$host/bin"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
rustc --edition 2024 --crate-type lib --emit obj --target x86_64-unknown-linux-gnu asm.rs -o "$out/boot.o"
LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-lld" -flavor gnu -m elf_x86_64 -e 0 -T boot.ld -o "$out/boot.elf" "$out/boot.o"
extract() {
    LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-objcopy" -O binary -j "$1" "$out/boot.elf" "$2"
    chmod 644 "$2"
}
extract .bonk_mbr ../crates/bonk-core/src/boot/mbr.bin
extract .bonk_test_vbr test-vbr.bin
