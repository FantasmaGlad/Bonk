#!/bin/sh
# Assemble le code d'amorçage BIOS avec la seule chaîne Rust (rustc, rust-lld, rust-objcopy) et écrit les
# binaires versionnés dans crates/bonk-core/src/boot/. La CI relance ce script et refuse tout écart.
set -eu
cd "$(dirname "$0")"
sysroot=$(rustc --print sysroot)
host=$(rustc -vV | sed -n 's/^host: //p')
bin="$sysroot/lib/rustlib/$host/bin"
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
rustc --edition 2024 --crate-type lib --emit obj --target x86_64-unknown-linux-gnu asm.rs -o "$out/boot.o"
LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-lld" -flavor gnu -m elf_x86_64 -e 0 -T boot.ld -o "$out/boot.elf" "$out/boot.o"
LD_LIBRARY_PATH="$sysroot/lib" "$bin/rust-objcopy" -O binary -j .bonk_mbr "$out/boot.elf" ../crates/bonk-core/src/boot/mbr.bin
chmod 644 ../crates/bonk-core/src/boot/mbr.bin
