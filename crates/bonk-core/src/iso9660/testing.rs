use super::*;

fn record(name: &str, extent: u32, size: u32, directory: bool) -> Vec<u8> {
    let short = name.to_ascii_uppercase();
    let short = if directory {
        short
    } else {
        format!("{short};1")
    };
    let nm = [
        b"NM".as_slice(),
        &[5 + name.len() as u8, 1, 0],
        name.as_bytes(),
    ]
    .concat();
    let mut r = vec![0u8; 33];
    r.extend(short.as_bytes());
    if short.len() % 2 == 0 {
        r.push(0);
    }
    r.extend(nm);
    r[0] = r.len() as u8;
    r[2..6].copy_from_slice(&extent.to_le_bytes());
    r[10..14].copy_from_slice(&size.to_le_bytes());
    r[25] = if directory { FLAG_DIRECTORY } else { 0 };
    r[32] = short.len() as u8;
    r
}

fn dot(extent: u32, name: u8) -> Vec<u8> {
    let mut r = vec![0u8; 34];
    r[0] = 34;
    r[2..6].copy_from_slice(&extent.to_le_bytes());
    r[10..14].copy_from_slice(&(SECTOR as u32).to_le_bytes());
    r[25] = FLAG_DIRECTORY;
    r[32] = 1;
    r[33] = name;
    r
}

/// One sector per directory, files as given (path, size); data is never read.
#[must_use]
pub fn image(label: &str, files: &[(&str, u32)]) -> Vec<u8> {
    let mut directories: Vec<String> = vec![String::new()];
    for (path, _) in files {
        let mut prefix = String::new();
        for part in path
            .split('/')
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            prefix = if prefix.is_empty() {
                part.to_owned()
            } else {
                format!("{prefix}/{part}")
            };
            if !directories.contains(&prefix) {
                directories.push(prefix.clone());
            }
        }
    }
    let first = 20u32;
    let extent = |dir: &str| first + directories.iter().position(|d| d == dir).unwrap_or(0) as u32;
    let mut image = vec![0u8; (first as usize + directories.len()) * SECTOR as usize];
    for dir in &directories {
        let mut sector = [dot(extent(dir), 0), dot(extent(dir), 1)].concat();
        let parent_of = |path: &str| path.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        for child in directories
            .iter()
            .filter(|d| !d.is_empty() && parent_of(d) == *dir)
        {
            let name = child.rsplit('/').next().unwrap_or(child);
            sector.extend(record(name, extent(child), SECTOR as u32, true));
        }
        for (path, size) in files.iter().filter(|(p, _)| parent_of(p) == *dir) {
            let name = path.rsplit('/').next().unwrap_or(path);
            sector.extend(record(name, 0, *size, false));
        }
        let at = extent(dir) as usize * SECTOR as usize;
        image[at..at + sector.len()].copy_from_slice(&sector);
    }
    let pvd = &mut image[(FIRST_DESCRIPTOR * SECTOR) as usize..][..SECTOR as usize];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[40..72].fill(b' ');
    pvd[40..40 + label.len()].copy_from_slice(label.as_bytes());
    pvd[156..190].copy_from_slice(&dot(first, 0));
    image
}
