//! Lecture du certificat d'un chargeur UEFI arbitraire : un refus, jamais une panique.
#![no_main]
use bonk_core::pe;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pe::signer(data);
    let _ = pe::is_x86_64(data);
});
