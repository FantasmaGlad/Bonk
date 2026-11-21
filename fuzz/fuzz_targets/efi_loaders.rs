#![no_main]

use bonk_core::analysis::HEAD_LEN;
use bonk_core::fat::Fat;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let head = &data[..data.len().min(HEAD_LEN)];
    let _ = bonk_core::efi::loaders(data, head);
    let _ = bonk_core::hybrid::hybrid_mbr(data);
    if let Some(fat) = Fat::open(data, 0) {
        let _ = fat.read("EFI/BOOT/BOOTX64.EFI", 1 << 20);
    }
});
