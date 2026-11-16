use std::collections::{BTreeMap, BTreeSet};

use super::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_LABEL, ATTR_LONG_NAME, ENTRY, FIXED_DATE, LONG_NAME_UNITS,
    MAX_NAME_UNITS, Row,
};
use crate::format::FormatError;
use crate::format::fat32::{self, ROOT_CLUSTER, narrow};
use crate::format::tree::{ChildKind, Children, NewFile, Placed};

pub(super) fn is_plain_short(name: &str) -> bool {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let good = |part: &str, max: usize| {
        part.len() <= max
            && part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    !base.is_empty() && good(base, 8) && good(ext, 3)
}

pub(super) fn short_chars(part: &str, max: usize) -> String {
    part.chars()
        .filter_map(|c| {
            let c = c.to_ascii_uppercase();
            (c.is_ascii_alphanumeric() || c == '_' || c == '-').then_some(c)
        })
        .take(max)
        .collect()
}

pub(super) fn pack_short(base: &str, ext: &str) -> [u8; 11] {
    let mut field = [b' '; 11];
    for (slot, byte) in field[..8].iter_mut().zip(base.bytes()) {
        *slot = byte;
    }
    for (slot, byte) in field[8..].iter_mut().zip(ext.bytes()) {
        *slot = byte;
    }
    field
}

/// The upper-cased name if valid, else `BASE~N.EXT`. The flag says a long entry must keep the exact name.
pub(super) fn short_name(
    name: &str,
    taken: &mut BTreeSet<[u8; 11]>,
) -> Result<([u8; 11], bool), FormatError> {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let upper = name.to_ascii_uppercase();
    if is_plain_short(&upper) {
        let (upper_base, upper_ext) = upper.rsplit_once('.').unwrap_or((&upper, ""));
        let field = pack_short(upper_base, upper_ext);
        if taken.insert(field) {
            return Ok((field, upper != name));
        }
    }
    let (clean_base, clean_ext) = (short_chars(base, 8), short_chars(ext, 3));
    for number in 1..1_000_000u32 {
        let suffix = format!("~{number}");
        let keep = 8 - suffix.len();
        let stem: String = clean_base.chars().take(keep).collect();
        let field = pack_short(&format!("{stem}{suffix}"), &clean_ext);
        if taken.insert(field) {
            return Ok((field, true));
        }
    }
    Err(FormatError::Name(format!("{name}: no free short name")))
}

pub(super) fn short_checksum(short: &[u8; 11]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, byte| sum.rotate_right(1).wrapping_add(*byte))
}

/// Last to first, as the format requires.
pub(super) fn long_entries(name: &str, short: &[u8; 11]) -> Result<Vec<[u8; ENTRY]>, FormatError> {
    let mut units: Vec<u16> = name.encode_utf16().collect();
    if units.len() > MAX_NAME_UNITS || name.chars().any(|c| "\"*/:<>?\\|".contains(c) || c < ' ') {
        return Err(FormatError::Name(format!("{name}: name refused by FAT32")));
    }
    if !units.len().is_multiple_of(LONG_NAME_UNITS) {
        units.push(0);
    }
    let count = units.len().div_ceil(LONG_NAME_UNITS);
    let checksum = short_checksum(short);
    let mut entries = Vec::with_capacity(count);
    for index in (0..count).rev() {
        let mut entry = [0u8; ENTRY];
        let order = u8::try_from(index + 1).map_err(|_| FormatError::Name(name.to_owned()))?;
        entry[0] = if index + 1 == count {
            order | 0x40
        } else {
            order
        };
        entry[11] = ATTR_LONG_NAME;
        entry[13] = checksum;
        let piece = &units[index * LONG_NAME_UNITS..units.len().min((index + 1) * LONG_NAME_UNITS)];
        let slots = (1..11)
            .step_by(2)
            .chain((14..26).step_by(2))
            .chain((28..32).step_by(2));
        for (position, slot) in slots.enumerate() {
            let unit = piece.get(position).copied().unwrap_or(0xFFFF);
            entry[slot..slot + 2].copy_from_slice(&unit.to_le_bytes());
        }
        entries.push(entry);
    }
    Ok(entries)
}

pub(super) fn short_entry(
    short: &[u8; 11],
    attributes: u8,
    cluster: u32,
    size: u32,
) -> [u8; ENTRY] {
    let mut entry = [0u8; ENTRY];
    entry[..11].copy_from_slice(short);
    entry[11] = attributes;
    for at in [16, 18, 24] {
        entry[at..at + 2].copy_from_slice(&FIXED_DATE.to_le_bytes());
    }
    entry[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    entry[26..28].copy_from_slice(&((cluster & 0xFFFF) as u16).to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

pub(super) const DOT: [u8; 11] = *b".          ";
pub(super) const DOT_DOT: [u8; 11] = *b"..         ";

pub(super) type Encoded = BTreeMap<Vec<String>, Vec<Row>>;
pub(super) type PlacedDirs = BTreeMap<Vec<String>, (u32, u32)>;

pub(super) fn encode_names(children: &Children) -> Result<Encoded, FormatError> {
    let mut encoded = Encoded::new();
    for (path, list) in children {
        let mut taken = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut rows = Vec::with_capacity(list.len());
        for (position, child) in list.iter().enumerate() {
            if !seen.insert(child.name.to_lowercase()) {
                return Err(FormatError::Name(format!(
                    "{}: two names FAT32 cannot tell apart",
                    child.name
                )));
            }
            let (short, needs_long) = short_name(&child.name, &mut taken)?;
            let long = if needs_long {
                long_entries(&child.name, &short)?
            } else {
                Vec::new()
            };
            rows.push((long, short, position));
        }
        encoded.insert(path.clone(), rows);
    }
    Ok(encoded)
}

pub(super) fn directory_bytes(
    label: Option<&str>,
    files: &[NewFile],
    children: &Children,
    encoded: &Encoded,
    placed_dirs: &PlacedDirs,
    placed_files: &[Placed],
) -> Result<Vec<(u32, u32, Vec<u8>)>, FormatError> {
    let mut directories = Vec::with_capacity(encoded.len());
    for (path, rows) in encoded {
        let (first, clusters) = placed_dirs[path];
        let mut bytes = Vec::new();
        if path.is_empty() {
            if let Some(label) = label {
                bytes.extend_from_slice(&short_entry(
                    &fat32::label_field(Some(label)),
                    ATTR_LABEL,
                    0,
                    0,
                ));
            }
        } else {
            let parent_first = match placed_dirs[&path[..path.len() - 1]].0 {
                // `..` pointing to the root is written 0.
                ROOT_CLUSTER => 0,
                other => other,
            };
            bytes.extend_from_slice(&short_entry(&DOT, ATTR_DIRECTORY, first, 0));
            bytes.extend_from_slice(&short_entry(&DOT_DOT, ATTR_DIRECTORY, parent_first, 0));
        }
        let list = &children[path];
        for (long, short, position) in rows {
            for entry in long {
                bytes.extend_from_slice(entry);
            }
            let row = match &list[*position].kind {
                ChildKind::File(index) => short_entry(
                    short,
                    ATTR_ARCHIVE,
                    placed_files[*index].first_cluster,
                    narrow(files[*index].size)?,
                ),
                ChildKind::Directory(child) => {
                    short_entry(short, ATTR_DIRECTORY, placed_dirs[child].0, 0)
                }
            };
            bytes.extend_from_slice(&row);
        }
        directories.push((first, clusters, bytes));
    }
    Ok(directories)
}
