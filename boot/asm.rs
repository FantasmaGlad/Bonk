//! Code d'amorçage BIOS, assemblé par l'assembleur de LLVM intégré à rustc : aucun outil hors de la
//! chaîne Rust. Chaque fichier `.s` place son code dans sa propre section, extraite par `assemble.sh`.

#![no_std]

core::arch::global_asm!(include_str!("mbr.s"), options(raw));
