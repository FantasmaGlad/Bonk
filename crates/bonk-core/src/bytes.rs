pub(crate) fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn array<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}

/// `None` when the field runs past the end: for structures whose length is not checked beforehand.
pub(crate) fn get_u16(b: &[u8], at: usize) -> Option<u16> {
    array(b, at).map(u16::from_le_bytes)
}

pub(crate) fn get_u32(b: &[u8], at: usize) -> Option<u32> {
    array(b, at).map(u32::from_le_bytes)
}

pub(crate) fn get_u64(b: &[u8], at: usize) -> Option<u64> {
    array(b, at).map(u64::from_le_bytes)
}

/// For a buffer whose length is already checked: out of range is a bug and panics.
pub(crate) fn u16_le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub(crate) fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub(crate) fn u64_le(b: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_bornee_et_petit_boutiste() {
        let b = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(get_u16(&b, 6), Some(0x0807));
        assert_eq!(get_u32(&b, 5), None);
        assert_eq!(get_u64(&b, 0), Some(u64_le(&b, 0)));
        assert_eq!(get_u32(&b, usize::MAX), None);
        assert_eq!(u32_le(&b, 1), 0x0504_0302);
    }
}
