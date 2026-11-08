#![no_std]

core::arch::global_asm!(include_str!("mbr.s"), options(raw));
core::arch::global_asm!(include_str!("vbr.s"), options(raw));
core::arch::global_asm!(include_str!("stage2.s"), options(raw));
core::arch::global_asm!(include_str!("test_vbr.s"), options(raw));
core::arch::global_asm!(include_str!("test_bootmgr.s"), options(raw));
