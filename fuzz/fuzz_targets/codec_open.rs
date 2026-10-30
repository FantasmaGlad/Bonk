//! `codec::open` puis lecture : un fichier compressé corrompu ou hostile doit finir en erreur, sans
//! panique, dans les trois décodeurs (gzip, xz, zstd).
#![no_main]

use std::io::{self, Cursor, Read};

use bonk_core::codec::open;
use libfuzzer_sys::fuzz_target;

/// Borne de sortie : une bombe de décompression est légitime pour le décodeur, elle ne doit pas
/// épuiser la mémoire ou le temps du fuzzeur.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let len = data.len() as u64;
    if let Ok(source) = open(Cursor::new(data.to_vec()), Some(len)) {
        let _ = io::copy(&mut source.reader.take(MAX_OUTPUT), &mut io::sink());
    }
});
