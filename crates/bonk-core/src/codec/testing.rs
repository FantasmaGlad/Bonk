use std::io::Write;

pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

/// Minimal valid xz: header, one stored LZMA2 block, index, footer. Hand-written from the xz
/// specification.
pub(crate) fn xz_stored(data: &[u8]) -> Vec<u8> {
    xz_stored_with_dictionary(data, 0x16)
}

/// `dictionary` is the LZMA2 property byte: 0x16 is 8 MiB, 40 is 4 GiB.
pub(crate) fn xz_stored_with_dictionary(data: &[u8], dictionary: u8) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        crate::partition::crc32(bytes)
    }
    fn varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while n >= 0x80 {
            out.push((n & 0x7F) as u8 | 0x80);
            n >>= 7;
        }
        out.push(n as u8);
        out
    }
    let mut out = vec![0xFD, b'7', b'z', b'X', b'Z', 0x00];
    let flags = [0x00u8, 0x00]; // check type: none
    out.extend_from_slice(&flags);
    out.extend_from_slice(&crc32(&flags).to_le_bytes());

    // Stored LZMA2 chunks of at most 64 KiB, then 0x00.
    let mut lzma2 = Vec::new();
    for (i, chunk) in data.chunks(1 << 16).enumerate() {
        lzma2.push(if i == 0 { 0x01 } else { 0x02 });
        lzma2.extend_from_slice(&((chunk.len() - 1) as u16).to_be_bytes());
        lzma2.extend_from_slice(chunk);
    }
    lzma2.push(0x00);

    // Block header: size, one LZMA2 filter (id 0x21, one property byte).
    let mut header = vec![0x00, 0x00, 0x21, 0x01, dictionary];
    while (header.len() + 4) % 4 != 0 {
        header.push(0x00);
    }
    header[0] = u8::try_from((header.len() + 4) / 4 - 1).unwrap_or(0);
    let header_crc = crc32(&header);
    let block_start = out.len();
    out.extend_from_slice(&header);
    out.extend_from_slice(&header_crc.to_le_bytes());
    out.extend_from_slice(&lzma2);
    // Unpadded size: header with its checksum, and data.
    let unpadded = out.len() - block_start;
    while (out.len() - block_start) % 4 != 0 {
        out.push(0x00);
    }
    let index_start = out.len();
    out.push(0x00);
    out.extend(varint(1));
    out.extend(varint(unpadded as u64));
    out.extend(varint(data.len() as u64));
    while (out.len() - index_start) % 4 != 0 {
        out.push(0x00);
    }
    let index_crc = crc32(&out[index_start..]);
    out.extend_from_slice(&index_crc.to_le_bytes());
    let backward = u32::try_from((out.len() - index_start) / 4 - 1).unwrap_or(0);
    let mut footer = Vec::new();
    footer.extend_from_slice(&backward.to_le_bytes());
    footer.extend_from_slice(&flags);
    out.extend_from_slice(&crc32(&footer).to_le_bytes());
    out.extend_from_slice(&footer);
    out.extend_from_slice(b"YZ");
    out
}
