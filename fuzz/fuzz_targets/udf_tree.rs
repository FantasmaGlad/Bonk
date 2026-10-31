//! Lecture UDF d'une image reçue d'ailleurs : ni panique, ni boucle, ni allocation sans borne.
#![no_main]
use bonk_core::udf::Udf;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(udf) = Udf::open(data) {
        let _ = udf.tree();
    }
});
