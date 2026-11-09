#![no_main]
use bonk_core::MemoryDevice;
use bonk_core::format::exfat_read::Reader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut bytes = data.to_vec();
    bytes.resize(data.len().div_ceil(512).max(1) * 512, 0);
    let sectors = bytes.len() as u64 / 512;
    if let Ok(dev) = MemoryDevice::from_bytes(bytes, 512)
        && let Ok(reader) = Reader::open(&dev, 0, sectors)
    {
        let _ = reader.label();
        let _ = reader.files();
    }
});
