#![no_main]

use bonk_core::analysis::analyze;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = analyze(data, Some(data.len() as u64));
    let _ = analyze(data, None);
});
