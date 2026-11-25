pub(super) fn short_name(part: &str) -> Option<[u8; 11]> {
    let (base, extension) = part.rsplit_once('.').unwrap_or((part, ""));
    let plain = |text: &str, max: usize| {
        !text.is_empty()
            && text.len() <= max
            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if !plain(base, 8) || !(extension.is_empty() || plain(extension, 3)) {
        return None;
    }
    let mut name = [b' '; 11];
    name[..base.len()].copy_from_slice(base.to_ascii_uppercase().as_bytes());
    name[8..8 + extension.len()].copy_from_slice(extension.to_ascii_uppercase().as_bytes());
    Some(name)
}

// Microsoft FAT specification, section 7: 13 UTF-16 units per entry, stored last first, tied to their short
// entry by its checksum.
#[derive(Debug, Default)]
pub(super) struct LongName {
    units: Vec<u16>,
    checksum: Option<u8>,
    next: u8,
}

impl LongName {
    pub(super) fn push(&mut self, raw: &[u8; 32]) {
        let order = raw[0] & 0x1F;
        if raw[0] & 0x40 != 0 {
            *self = Self {
                units: Vec::new(),
                checksum: Some(raw[13]),
                next: order,
            };
        }
        if order == 0 || order != self.next || self.checksum != Some(raw[13]) {
            *self = Self::default();
            return;
        }
        let mut units: Vec<u16> = (1..11)
            .step_by(2)
            .chain((14..26).step_by(2))
            .chain((28..32).step_by(2))
            .map(|at| u16::from_le_bytes([raw[at], raw[at + 1]]))
            .collect();
        units.append(&mut self.units);
        self.units = units;
        self.next = order - 1;
    }

    pub(super) fn take(&mut self, short: &[u8]) -> Option<String> {
        let done = std::mem::take(self);
        if done.next != 0 || done.checksum != Some(checksum(short)) {
            return None;
        }
        let end = done
            .units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(done.units.len());
        String::from_utf16(&done.units[..end]).ok()
    }
}

fn checksum(short: &[u8]) -> u8 {
    short[..11]
        .iter()
        .fold(0u8, |sum, byte| sum.rotate_right(1).wrapping_add(*byte))
}
