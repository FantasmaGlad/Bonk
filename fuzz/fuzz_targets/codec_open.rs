#![no_main]

use std::io::{self, Cursor, Read};

use bonk_core::codec::open;
use libfuzzer_sys::fuzz_target;

/// Output cap: a decompression bomb is legitimate input, it must not exhaust the fuzzer's memory or time.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let len = data.len() as u64;
    if let Ok(source) = open(Cursor::new(data.to_vec()), Some(len)) {
        let _ = io::copy(&mut source.reader.take(MAX_OUTPUT), &mut io::sink());
    }
});
