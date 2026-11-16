use std::collections::{BTreeMap, BTreeSet};

use super::entries::{entries_for, entry_set, name_units, system_entries};
use super::{Names, narrow};
use crate::format::FormatError;
use crate::format::exfat::{ENTRY, Geometry, MAX_DIRECTORY_BYTES};
use crate::format::tree::{ChildKind, Children, NewFile, Placed};

pub(super) fn name_units_by_directory(
    children: &Children,
    upcase: &[u16],
) -> Result<Names, FormatError> {
    let mut names = Names::new();
    for (path, list) in children {
        let mut seen = BTreeSet::new();
        let mut units_of = Vec::with_capacity(list.len());
        for child in list {
            let units = name_units(&child.name)?;
            let key: Vec<u16> = units.iter().map(|u| upcase[usize::from(*u)]).collect();
            if !seen.insert(key) {
                return Err(FormatError::Name(format!(
                    "{}: two names exFAT cannot tell apart",
                    child.name
                )));
            }
            units_of.push(units);
        }
        names.insert(path.clone(), units_of);
    }
    Ok(names)
}

pub(super) fn place_directories(
    names: &Names,
    geo: &Geometry,
    labelled: bool,
    next: &mut u64,
) -> Result<BTreeMap<Vec<String>, Placed>, FormatError> {
    let mut placed = BTreeMap::new();
    for (path, units_of) in names {
        let mut entries: usize = units_of.iter().map(|u| entries_for(u)).sum();
        if path.is_empty() {
            entries += if labelled { 3 } else { 2 };
        }
        let bytes = (entries * ENTRY) as u64;
        if bytes > MAX_DIRECTORY_BYTES {
            return Err(FormatError::TooLarge(format!(
                "/{}: directory larger than exFAT allows",
                path.join("/")
            )));
        }
        let clusters = geo.clusters_for(bytes);
        placed.insert(
            path.clone(),
            Placed {
                first_cluster: narrow(*next)?,
                clusters: narrow(clusters)?,
            },
        );
        *next += clusters;
    }
    Ok(placed)
}

pub(super) struct Tree<'a> {
    pub(super) geo: &'a Geometry,
    pub(super) label: Option<&'a str>,
    pub(super) upcase: &'a [u16],
    pub(super) children: &'a Children,
    pub(super) files: &'a [NewFile],
    pub(super) placed_dirs: &'a BTreeMap<Vec<String>, Placed>,
    pub(super) placed_files: &'a [Placed],
}

impl Tree<'_> {
    pub(super) fn directory(&self, path: &Vec<String>, units_of: &[Vec<u16>]) -> Vec<u8> {
        let cluster_bytes = u64::from(self.geo.cluster_bytes);
        let mut bytes = if path.is_empty() {
            system_entries(self.geo, self.label)
        } else {
            Vec::new()
        };
        for (child, units) in self.children[path].iter().zip(units_of) {
            bytes.extend(match &child.kind {
                ChildKind::File(index) => entry_set(
                    units,
                    self.upcase,
                    false,
                    self.placed_files[*index],
                    self.files[*index].size,
                ),
                ChildKind::Directory(sub) => {
                    let dir = self.placed_dirs[sub];
                    entry_set(
                        units,
                        self.upcase,
                        true,
                        dir,
                        u64::from(dir.clusters) * cluster_bytes,
                    )
                }
            });
        }
        let placed = self.placed_dirs[path];
        bytes.resize((u64::from(placed.clusters) * cluster_bytes) as usize, 0);
        bytes
    }
}
