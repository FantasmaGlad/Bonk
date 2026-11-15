use std::collections::BTreeMap;
use std::io::{self, Read};

use sha2::{Digest, Sha256};

use super::FormatError;
use crate::block::BlockDevice;

const IO_CHUNK: usize = 4 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFile {
    pub path: Vec<String>,
    pub size: u64,
}

#[derive(Debug)]
pub(super) struct Child {
    pub(super) name: String,
    pub(super) kind: ChildKind,
}

#[derive(Debug)]
pub(super) enum ChildKind {
    File(usize),
    Directory(Vec<String>),
}

/// Each directory and its content; the root is the empty path.
pub(super) type Children = BTreeMap<Vec<String>, Vec<Child>>;

fn add_directory(children: &mut Children, path: &[String]) {
    for depth in 1..=path.len() {
        let here = path[..depth].to_vec();
        if !children.contains_key(&here) {
            children.insert(here.clone(), Vec::new());
            if let Some(parent) = children.get_mut(&path[..depth - 1]) {
                parent.push(Child {
                    name: path[depth - 1].clone(),
                    kind: ChildKind::Directory(here),
                });
            }
        }
    }
}

/// A file over `max_file_bytes` is refused, naming `filesystem`.
pub(super) fn collect_children(
    files: &[NewFile],
    extra_directories: &[Vec<String>],
    max_file_bytes: u64,
    filesystem: &str,
) -> Result<Children, FormatError> {
    let mut children = Children::new();
    children.insert(Vec::new(), Vec::new());
    for path in extra_directories {
        add_directory(&mut children, path);
    }
    for (index, file) in files.iter().enumerate() {
        if file.size > max_file_bytes {
            return Err(FormatError::TooLarge(format!(
                "{}: {} bytes, {filesystem} stops at {max_file_bytes}",
                file.path.join("/"),
                file.size
            )));
        }
        let (name, parent) = file
            .path
            .split_last()
            .ok_or_else(|| FormatError::Name("empty path".to_owned()))?;
        add_directory(&mut children, parent);
        if let Some(list) = children.get_mut(parent) {
            list.push(Child {
                name: name.clone(),
                kind: ChildKind::File(index),
            });
        }
    }
    Ok(children)
}

/// A contiguous run of clusters.
#[derive(Debug, Clone, Copy)]
pub(super) struct Placed {
    pub(super) first_cluster: u32,
    pub(super) clusters: u32,
}

/// Files one after the other from `next`, in their order; an empty file takes no cluster. `narrow` names the
/// filesystem when a number overflows.
pub(super) fn place_files(
    files: &[NewFile],
    cluster_bytes: u64,
    next: &mut u64,
    narrow: fn(u64) -> Result<u32, FormatError>,
) -> Result<Vec<Placed>, FormatError> {
    let mut placed = Vec::with_capacity(files.len());
    for file in files {
        let clusters = file.size.div_ceil(cluster_bytes);
        placed.push(Placed {
            first_cluster: if clusters == 0 { 0 } else { narrow(*next)? },
            clusters: narrow(clusters)?,
        });
        *next += clusters;
    }
    Ok(placed)
}

/// Refused before the first write: `used` clusters do not fit in the `available` ones.
pub(super) fn check_room(used: u64, available: u64, cluster_bytes: u64) -> Result<(), FormatError> {
    if used > available {
        return Err(FormatError::TooSmall(format!(
            "{} bytes to store: the drive only has {} data bytes",
            used * cluster_bytes,
            available * cluster_bytes
        )));
    }
    Ok(())
}

/// Copies `size` bytes contiguously and returns their SHA-256. `advance` gets the bytes of each block and
/// returns false to stop.
pub(super) fn write_contiguous(
    dev: &mut dyn BlockDevice,
    offset: u64,
    size: u64,
    reader: &mut dyn Read,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<String, FormatError> {
    let block = dev.block_size() as usize;
    let mut hasher = Sha256::new();
    let mut done = 0;
    let mut buffer = vec![0u8; IO_CHUNK];
    while done < size {
        let want = (size - done).min(IO_CHUNK as u64) as usize;
        reader
            .read_exact(&mut buffer[..want])
            .map_err(FormatError::Source)?;
        hasher.update(&buffer[..want]);
        let padded = want.next_multiple_of(block);
        buffer[want..padded].fill(0);
        dev.write_at(offset + done, &buffer[..padded])?;
        done += want as u64;
        if !advance(want as u64) {
            return Err(FormatError::Source(io::ErrorKind::Interrupted.into()));
        }
    }
    Ok(crate::write::hex(&hasher.finalize()))
}

pub(super) fn verify_contiguous(
    dev: &dyn BlockDevice,
    offset: u64,
    size: u64,
    expected: &str,
    what: &str,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<(), FormatError> {
    let block = dev.block_size() as usize;
    let mut hasher = Sha256::new();
    let mut done = 0;
    let mut buffer = vec![0u8; IO_CHUNK];
    while done < size {
        let want = (size - done).min(IO_CHUNK as u64) as usize;
        let padded = want.next_multiple_of(block);
        dev.read_at(offset + done, &mut buffer[..padded])?;
        hasher.update(&buffer[..want]);
        done += want as u64;
        if !advance(want as u64) {
            return Err(FormatError::Source(io::ErrorKind::Interrupted.into()));
        }
    }
    if crate::write::hex(&hasher.finalize()) == expected {
        Ok(())
    } else {
        Err(FormatError::Mismatch(format!(
            "{what}: the read-back differs from what was written"
        )))
    }
}
