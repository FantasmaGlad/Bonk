use super::*;

pub(in crate::format) fn name_units(name: &str) -> Result<Vec<u16>, FormatError> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let forbidden = |c: char| c < ' ' || "\"*/:<>?\\|".contains(c);
    if units.is_empty() || units.len() > MAX_NAME_UNITS || name.chars().any(forbidden) {
        return Err(FormatError::Name(format!("{name}: name refused by exFAT")));
    }
    Ok(units)
}

pub(in crate::format) fn entries_for(units: &[u16]) -> usize {
    2 + units.len().div_ceil(NAME_UNITS)
}

pub(in crate::format) fn entry_set(
    units: &[u16],
    upcase: &[u16],
    directory: bool,
    placed: Placed,
    data_length: u64,
) -> Vec<u8> {
    let count = entries_for(units);
    let mut set = vec![0u8; count * ENTRY];
    set[0] = ENTRY_FILE;
    set[1] = (count - 1) as u8;
    put_u16(
        &mut set,
        4,
        if directory {
            ATTR_DIRECTORY
        } else {
            ATTR_ARCHIVE
        },
    );
    for at in [8, 12, 16] {
        put_u32(&mut set, at, FIXED_TIMESTAMP);
    }
    let stream = &mut set[ENTRY..2 * ENTRY];
    stream[0] = ENTRY_STREAM;
    stream[1] = if placed.clusters == 0 {
        ALLOCATION_POSSIBLE
    } else {
        ALLOCATION_POSSIBLE | NO_FAT_CHAIN
    };
    stream[3] = units.len() as u8;
    let upcased: Vec<u8> = units
        .iter()
        .flat_map(|unit| upcase[usize::from(*unit)].to_le_bytes())
        .collect();
    put_u16(stream, 4, checksum16(&upcased, &[]));
    put_u64(stream, 8, data_length);
    put_u32(stream, 20, placed.first_cluster);
    put_u64(stream, 24, data_length);
    for (index, piece) in units.chunks(NAME_UNITS).enumerate() {
        let entry = &mut set[(2 + index) * ENTRY..(3 + index) * ENTRY];
        entry[0] = ENTRY_NAME;
        for (position, unit) in piece.iter().enumerate() {
            put_u16(entry, 2 + position * 2, *unit);
        }
    }
    let sum = checksum16(&set, &[2, 3]);
    put_u16(&mut set, 2, sum);
    set
}

/// Entries the root must start with.
pub(super) fn system_entries(geo: &Geometry, label: Option<&str>) -> Vec<u8> {
    let mut bytes = vec![0u8; if label.is_some() { 3 } else { 2 } * ENTRY];
    let bitmap = &mut bytes[..ENTRY];
    bitmap[0] = ENTRY_BITMAP;
    put_u32(bitmap, 20, geo.bitmap_cluster());
    put_u64(bitmap, 24, u64::from(geo.bitmap_bytes()));
    let upcase = &mut bytes[ENTRY..2 * ENTRY];
    upcase[0] = ENTRY_UPCASE;
    put_u32(upcase, 4, checksum32(UPCASE, &[]));
    put_u32(upcase, 20, geo.upcase_cluster());
    put_u64(upcase, 24, UPCASE.len() as u64);
    if let Some(label) = label {
        let entry = &mut bytes[2 * ENTRY..];
        entry[0] = ENTRY_LABEL;
        let units: Vec<u16> = label.encode_utf16().take(11).collect();
        entry[1] = units.len() as u8;
        for (i, unit) in units.iter().enumerate() {
            put_u16(entry, 2 + i * 2, *unit);
        }
    }
    bytes
}
