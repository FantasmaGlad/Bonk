#![no_main]
use bonk_core::wim;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 208 {
        let (header, lookup) = data.split_at(208);
        let _ = wim::split(header, lookup, u64::MAX / 2, 1 << 20);
    }
});
