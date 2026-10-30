//! `analysis::analyze` sur des octets quelconques : aucune panique, quelles que soient les valeurs lues
//! dans les tables (elles viennent de l'image).
#![no_main]

use bonk_core::analysis::analyze;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = analyze(data, Some(data.len() as u64));
    let _ = analyze(data, None);
});
