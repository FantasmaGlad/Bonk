use super::raw::*;

fn tag(sector: &mut [u8], id: u16) {
    sector[0..2].copy_from_slice(&id.to_le_bytes());
    sector[2] = 2;
    let sum = sector[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |s, (_, b)| s.wrapping_add(*b));
    sector[4] = sum;
}

fn fid(characteristics: u8, name: &str, block: u32) -> Vec<u8> {
    let mut encoded = vec![8];
    encoded.extend(name.bytes());
    let name_len = if name.is_empty() { 0 } else { encoded.len() };
    let mut record = vec![0; (38 + name_len + 3) & !3];
    record[18] = characteristics;
    record[19] = name_len as u8;
    record[20..24].copy_from_slice(&2048u32.to_le_bytes());
    record[24..28].copy_from_slice(&block.to_le_bytes());
    record[38..38 + name_len].copy_from_slice(&encoded[..name_len]);
    tag(&mut record, TAG_FILE_ID);
    record
}

const P: usize = 300;
const BLOCK: usize = 2048;

fn block(b: usize) -> usize {
    (P + b) * BLOCK
}

/// Anchor at sector 256 pointing to the descriptor sequence at 257: partition at block `P`, one
/// logical volume, terminator.
fn volume_descriptors(img: &mut [u8]) {
    let s = 256 * BLOCK;
    img[s + 16..s + 20].copy_from_slice(&(16 * 2048u32).to_le_bytes());
    img[s + 20..s + 24].copy_from_slice(&257u32.to_le_bytes());
    tag(&mut img[s..s + BLOCK], TAG_ANCHOR);
    let s = 257 * BLOCK;
    img[s + 188..s + 192].copy_from_slice(&(P as u32).to_le_bytes());
    tag(&mut img[s..s + BLOCK], TAG_PARTITION);
    let s = 258 * BLOCK;
    img[s + 212..s + 216].copy_from_slice(&2048u32.to_le_bytes());
    img[s + 268..s + 272].copy_from_slice(&1u32.to_le_bytes());
    img[s + 440] = 1;
    tag(&mut img[s..s + BLOCK], TAG_LOGICAL_VOLUME);
    tag(&mut img[259 * BLOCK..260 * BLOCK], TAG_TERMINATOR);
}

fn file_set(img: &mut [u8], root: u32) {
    let s = block(0);
    img[s + 404..s + 408].copy_from_slice(&root.to_le_bytes());
    tag(&mut img[s..s + BLOCK], TAG_FILE_SET);
}

/// File entry at `entry` whose single extent of `bytes` starts at block `data`.
fn extent_entry(img: &mut [u8], entry: usize, data: usize, bytes: &[u8], directory: bool) {
    img[block(data)..block(data) + bytes.len()].copy_from_slice(bytes);
    let s = block(entry);
    if directory {
        img[s + 27] = ICB_DIRECTORY;
    }
    img[s + 56..s + 64].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    img[s + 172..s + 176].copy_from_slice(&8u32.to_le_bytes());
    img[s + 176..s + 180].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    img[s + 180..s + 184].copy_from_slice(&(data as u32).to_le_bytes());
    tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
}

type Dirs = std::collections::BTreeMap<String, Vec<(String, Option<usize>)>>;

/// Each directory's children: a name and, for a file, its index in `files`.
fn tree(files: &[(&str, &[u8])]) -> Dirs {
    let mut dirs = Dirs::new();
    dirs.insert(String::new(), Vec::new());
    for (index, (path, _)) in files.iter().enumerate() {
        let parts: Vec<&str> = path.split('/').collect();
        for depth in 0..parts.len() {
            let leaf = (depth + 1 == parts.len()).then_some(index);
            let entry = (parts[depth].to_owned(), leaf);
            let list = dirs.entry(parts[..depth].join("/")).or_default();
            if !list.contains(&entry) {
                list.push(entry);
            }
        }
    }
    dirs
}

/// Files given by path, one extent each.
#[must_use]
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    let dirs = tree(files);
    // Block 0: file set; then each directory's entry and data; then the files.
    let mut next = 1;
    let mut dir_blocks = std::collections::BTreeMap::new();
    for (path, list) in &dirs {
        dir_blocks.insert(path.clone(), next);
        next += 1 + ((list.len() + 1) * 64 * 8).div_ceil(BLOCK);
    }
    let mut file_blocks = Vec::new();
    for (_, data) in files {
        file_blocks.push(next);
        next += 1 + data.len().div_ceil(BLOCK).max(1);
    }
    let mut img = vec![0u8; (P + next + 1) * BLOCK];
    // Volume recognition: what image analysis checks for ISO and UDF.
    for (sector, id, kind) in [(16, b"CD001", 1u8), (17, b"BEA01", 0), (18, b"NSR02", 0)] {
        let s = sector * BLOCK;
        img[s] = kind;
        img[s + 1..s + 6].copy_from_slice(id);
        img[s + 6] = 1;
    }
    volume_descriptors(&mut img);
    file_set(&mut img, dir_blocks[""] as u32);
    for (path, list) in &dirs {
        let entry = dir_blocks[path];
        let mut records = vec![fid(FID_PARENT | FID_DIRECTORY, "", entry as u32)];
        for (name, file) in list {
            records.push(match file {
                Some(index) => fid(0, name, file_blocks[*index] as u32),
                None if path.is_empty() => fid(FID_DIRECTORY, name, dir_blocks[name] as u32),
                None => fid(
                    FID_DIRECTORY,
                    name,
                    dir_blocks[&format!("{path}/{name}")] as u32,
                ),
            });
        }
        extent_entry(&mut img, entry, entry + 1, &records.concat(), true);
    }
    for ((_, data), first) in files.iter().zip(&file_blocks) {
        extent_entry(&mut img, *first, first + 1, data, false);
    }
    img
}

#[must_use]
pub fn image() -> Vec<u8> {
    let mut img = vec![0u8; 700 * BLOCK];
    volume_descriptors(&mut img);
    file_set(&mut img, 1);
    let root = [
        fid(FID_PARENT | FID_DIRECTORY, "", 1),
        fid(FID_DIRECTORY, "SOURCES", 3),
        fid(0, "readme.txt", 5),
        fid(FID_DIRECTORY, "vide", 6),
        fid(FID_DELETED, "efface.txt", 5),
    ];
    extent_entry(&mut img, 1, 2, &root.concat(), true);
    let sources = [
        fid(FID_PARENT | FID_DIRECTORY, "", 1),
        fid(0, "boot.wim", 8),
    ];
    extent_entry(&mut img, 3, 4, &sources.concat(), true);
    let s = block(5);
    img[s + 34] = 3;
    img[s + 56..s + 64].copy_from_slice(&5u64.to_le_bytes());
    img[s + 172..s + 176].copy_from_slice(&5u32.to_le_bytes());
    img[s + 176..s + 181].copy_from_slice(b"salut");
    tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
    extent_entry(
        &mut img,
        6,
        7,
        &fid(FID_PARENT | FID_DIRECTORY, "", 1),
        true,
    );
    // 3000-byte file in two separate extents, through an extended entry.
    let s = block(8);
    img[s + 56..s + 64].copy_from_slice(&3000u64.to_le_bytes());
    img[s + 212..s + 216].copy_from_slice(&16u32.to_le_bytes());
    img[s + 216..s + 220].copy_from_slice(&2048u32.to_le_bytes());
    img[s + 220..s + 224].copy_from_slice(&20u32.to_le_bytes());
    img[s + 224..s + 228].copy_from_slice(&952u32.to_le_bytes());
    img[s + 228..s + 232].copy_from_slice(&30u32.to_le_bytes());
    tag(&mut img[s..s + BLOCK], TAG_EXTENDED_FILE_ENTRY);
    img[block(20)..block(20) + 2048].fill(0xAA);
    img[block(30)..block(30) + 952].fill(0xBB);
    img
}
