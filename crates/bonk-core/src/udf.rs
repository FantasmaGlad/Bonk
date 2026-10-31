//! Lecture d'un système de fichiers UDF (ECMA-167, profil OSTA), sans monter l'image : les ISO récentes de
//! Windows n'ont presque rien en ISO 9660. Une image vient de l'extérieur : chaque champ est borné, et une
//! structure incohérente devient une erreur, jamais une panique ni une boucle.

use std::fmt;
use std::io;

/// Un secteur de support optique. L'ancre principale est toujours au secteur 256.
const SECTOR: u64 = 2048;
const ANCHOR_SECTOR: u64 = 256;
/// Limites contre une image construite pour épuiser la mémoire ou tourner sans fin.
const MAX_DESCRIPTORS: u64 = 4096;
const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_DIRECTORY_BYTES: u64 = 64 << 20;

const TAG_PARTITION: u16 = 5;
const TAG_LOGICAL_VOLUME: u16 = 6;
const TAG_TERMINATOR: u16 = 8;
const TAG_ANCHOR: u16 = 2;
const TAG_FILE_SET: u16 = 256;
const TAG_FILE_ID: u16 = 257;
const TAG_FILE_ENTRY: u16 = 261;
const TAG_EXTENDED_FILE_ENTRY: u16 = 266;

const ICB_DIRECTORY: u8 = 4;
const FID_DIRECTORY: u8 = 0x02;
const FID_DELETED: u8 = 0x04;
const FID_PARENT: u8 = 0x08;

/// Lecture à une position donnée : l'image peut être lue par plusieurs fichiers à la fois, sans curseur.
pub trait ReadAt {
    /// # Errors
    /// Lecture impossible, ou au-delà de la fin.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl ReadAt for [u8] {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let end = start
            .checked_add(buf.len())
            .filter(|end| *end <= self.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(&self[start..end]);
        Ok(())
    }
}

#[derive(Debug)]
pub enum UdfError {
    /// Pas de système de fichiers UDF ici.
    NotUdf,
    /// UDF, mais une structure incohérente ou que Bonk ne lit pas.
    Malformed(&'static str),
    Io(io::Error),
}

impl fmt::Display for UdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUdf => f.write_str("aucun système de fichiers UDF"),
            Self::Malformed(what) => write!(f, "UDF illisible : {what}"),
            Self::Io(err) => write!(f, "lecture de l'image : {err}"),
        }
    }
}

impl std::error::Error for UdfError {}

impl From<io::Error> for UdfError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Un morceau de fichier : position dans l'image et longueur, en octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub len: u64,
}

/// Données d'un fichier : dans des morceaux de l'image, ou directement dans son entrée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Extents(Vec<Extent>),
    Inline(Vec<u8>),
}

/// Un fichier de l'image, avec son chemin depuis la racine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: Vec<String>,
    pub size: u64,
    pub content: Content,
}

/// Arbre d'une image : ses fichiers, et ses dossiers (les vides aussi).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub files: Vec<File>,
    pub directories: Vec<Vec<String>>,
}

impl Tree {
    /// Le fichier de ce chemin, sans tenir compte de la casse.
    #[must_use]
    pub fn find(&self, path: &[&str]) -> Option<&File> {
        self.files.iter().find(|file| {
            file.path.len() == path.len()
                && file
                    .path
                    .iter()
                    .zip(path)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
        })
    }
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, UdfError> {
    let bytes = data
        .get(at..at + 2)
        .ok_or(UdfError::Malformed("champ hors du descripteur"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, UdfError> {
    let bytes = data
        .get(at..at + 4)
        .ok_or(UdfError::Malformed("champ hors du descripteur"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn u64_at(data: &[u8], at: usize) -> Result<u64, UdfError> {
    let low = u64::from(u32_at(data, at)?);
    let high = u64::from(u32_at(data, at + 4)?);
    Ok(low | (high << 32))
}

/// Identifiant d'un descripteur dont l'étiquette est valide : somme de contrôle et version.
fn tag_id(sector: &[u8]) -> Option<u16> {
    let head = sector.get(..16)?;
    let sum = head
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
    (sum == head[4]).then(|| u16::from_le_bytes([head[0], head[1]]))
}

/// Position d'un bloc logique, la partition étant unique dans ce que Bonk lit.
#[derive(Clone, Copy)]
struct LongAd {
    block: u32,
}

fn long_ad(data: &[u8], at: usize) -> Result<LongAd, UdfError> {
    Ok(LongAd {
        block: u32_at(data, at + 4)?,
    })
}

/// Le type d'un morceau tient dans les deux bits hauts de sa longueur ; 0 : enregistré.
fn recorded_len(raw: u32) -> Result<u64, UdfError> {
    if raw >> 30 == 0 {
        Ok(u64::from(raw))
    } else {
        Err(UdfError::Malformed("morceau de fichier non enregistré"))
    }
}

/// Décode un nom : premier octet 8 pour du Latin-1, 16 pour de l'UTF-16 gros-boutiste.
fn decode_name(raw: &[u8]) -> Result<String, UdfError> {
    let name = match raw.split_first() {
        Some((8, rest)) => rest.iter().map(|b| char::from(*b)).collect(),
        Some((16, rest)) if rest.len().is_multiple_of(2) => {
            let units = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c));
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| UdfError::Malformed("nom de fichier invalide"))?
        }
        _ => return Err(UdfError::Malformed("nom de fichier invalide")),
    };
    // Un nom vient de l'image et deviendra un chemin : aucun moyen d'en sortir.
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(UdfError::Malformed("nom de fichier invalide"));
    }
    Ok(name)
}

/// Une entrée de fichier lue : genre, taille, données.
struct Entry {
    directory: bool,
    size: u64,
    content: Content,
}

/// Lecteur d'une image UDF à une seule partition de type 1, le cas des ISO de Windows.
pub struct Udf<'a, R: ReadAt + ?Sized> {
    reader: &'a R,
    partition_start: u64,
    root: LongAd,
    block: u64,
}

impl<'a, R: ReadAt + ?Sized> Udf<'a, R> {
    /// # Errors
    /// [`UdfError::NotUdf`] sans ancre ; [`UdfError::Malformed`] pour une structure incohérente.
    pub fn open(reader: &'a R) -> Result<Self, UdfError> {
        let mut anchor = [0; SECTOR as usize];
        if reader
            .read_exact_at(ANCHOR_SECTOR * SECTOR, &mut anchor)
            .is_err()
            || tag_id(&anchor) != Some(TAG_ANCHOR)
        {
            return Err(UdfError::NotUdf);
        }
        let sequence_len = u64::from(u32_at(&anchor, 16)?);
        let sequence_start = u64::from(u32_at(&anchor, 20)?);
        let mut partition = None;
        let mut volume = None;
        for index in 0..(sequence_len / SECTOR).min(MAX_DESCRIPTORS) {
            let mut sector = [0; SECTOR as usize];
            reader.read_exact_at((sequence_start + index) * SECTOR, &mut sector)?;
            match tag_id(&sector) {
                Some(TAG_PARTITION) => partition = Some(u64::from(u32_at(&sector, 188)?)),
                Some(TAG_LOGICAL_VOLUME) => {
                    let block = u32_at(&sector, 212)?;
                    let maps = u32_at(&sector, 268)?;
                    let map_kind = *sector
                        .get(440)
                        .ok_or(UdfError::Malformed("table de partitions"))?;
                    if block != SECTOR as u32 || maps != 1 || map_kind != 1 {
                        return Err(UdfError::Malformed("forme de volume non prise en charge"));
                    }
                    volume = Some(long_ad(&sector, 248)?);
                }
                Some(TAG_TERMINATOR) => break,
                _ => {}
            }
        }
        let partition_start = partition.ok_or(UdfError::Malformed("partition absente"))?;
        let file_set = volume.ok_or(UdfError::Malformed("volume logique absent"))?;
        let mut udf = Self {
            reader,
            partition_start,
            root: file_set,
            block: SECTOR,
        };
        let mut sector = [0; SECTOR as usize];
        reader.read_exact_at(udf.block_offset(file_set.block), &mut sector)?;
        if tag_id(&sector) != Some(TAG_FILE_SET) {
            return Err(UdfError::Malformed("descripteur du jeu de fichiers"));
        }
        udf.root = long_ad(&sector, 400)?;
        Ok(udf)
    }

    fn block_offset(&self, block: u32) -> u64 {
        (self.partition_start + u64::from(block)) * self.block
    }

    fn read_entry(&self, at: LongAd) -> Result<Entry, UdfError> {
        let mut sector = [0; SECTOR as usize];
        self.reader
            .read_exact_at(self.block_offset(at.block), &mut sector)?;
        let (ea_at, ad_at, base) = match tag_id(&sector) {
            Some(TAG_FILE_ENTRY) => (168, 172, 176),
            Some(TAG_EXTENDED_FILE_ENTRY) => (208, 212, 216),
            _ => return Err(UdfError::Malformed("entrée de fichier attendue")),
        };
        let directory = sector[27] == ICB_DIRECTORY;
        let flags = u16_at(&sector, 34)?;
        let size = u64_at(&sector, 56)?;
        let ea_len = u32_at(&sector, ea_at)? as usize;
        let ad_len = u32_at(&sector, ad_at)? as usize;
        let start = base + ea_len;
        let descriptors = start
            .checked_add(ad_len)
            .and_then(|end| sector.get(start..end))
            .ok_or(UdfError::Malformed("morceaux hors de l'entrée"))?;
        let content = match flags & 7 {
            0 => Content::Extents(self.extents(descriptors, 8, |d| u32_at(d, 4))?),
            1 => Content::Extents(self.extents(descriptors, 16, |d| u32_at(d, 4))?),
            3 => Content::Inline(descriptors.to_vec()),
            _ => return Err(UdfError::Malformed("forme de morceaux non prise en charge")),
        };
        let held = match &content {
            Content::Extents(extents) => extents.iter().map(|extent| extent.len).sum(),
            Content::Inline(bytes) => bytes.len() as u64,
        };
        if held < size {
            return Err(UdfError::Malformed("fichier plus long que ses morceaux"));
        }
        Ok(Entry {
            directory,
            size,
            content,
        })
    }

    fn extents(
        &self,
        descriptors: &[u8],
        width: usize,
        block: impl Fn(&[u8]) -> Result<u32, UdfError>,
    ) -> Result<Vec<Extent>, UdfError> {
        let mut extents = Vec::new();
        for descriptor in descriptors.chunks_exact(width) {
            let raw = u32_at(descriptor, 0)?;
            if raw == 0 {
                break;
            }
            extents.push(Extent {
                offset: self.block_offset(block(descriptor)?),
                len: recorded_len(raw)?,
            });
        }
        Ok(extents)
    }

    fn directory_bytes(&self, entry: &Entry) -> Result<Vec<u8>, UdfError> {
        if entry.size > MAX_DIRECTORY_BYTES {
            return Err(UdfError::Malformed("dossier trop grand"));
        }
        let mut bytes = vec![0; entry.size as usize];
        read_content(self.reader, &entry.content, 0, &mut bytes)?;
        Ok(bytes)
    }

    /// L'arbre complet de l'image.
    ///
    /// # Errors
    /// Structure incohérente, image illisible, ou arbre au-delà des limites.
    pub fn tree(&self) -> Result<Tree, UdfError> {
        let mut tree = Tree::default();
        let mut pending = vec![(self.root, Vec::new())];
        while let Some((at, path)) = pending.pop() {
            let directory = self.read_entry(at)?;
            if !directory.directory {
                return Err(UdfError::Malformed("dossier attendu"));
            }
            let bytes = self.directory_bytes(&directory)?;
            let mut position = 0;
            while position + 38 <= bytes.len() {
                let record = &bytes[position..];
                if tag_id(record) != Some(TAG_FILE_ID) {
                    return Err(UdfError::Malformed("entrée de dossier"));
                }
                let characteristics = record[18];
                let name_len = usize::from(record[19]);
                let use_len = usize::from(u16_at(record, 36)?);
                let total = (38 + use_len + name_len + 3) & !3;
                if total > record.len() {
                    return Err(UdfError::Malformed("entrée de dossier tronquée"));
                }
                position += total;
                if characteristics & (FID_PARENT | FID_DELETED) != 0 {
                    continue;
                }
                if tree.files.len() + tree.directories.len() >= MAX_ENTRIES {
                    return Err(UdfError::Malformed("trop de fichiers"));
                }
                let mut child = path.clone();
                child.push(decode_name(&record[38 + use_len..38 + use_len + name_len])?);
                let icb = long_ad(record, 20)?;
                if characteristics & FID_DIRECTORY != 0 {
                    if child.len() > MAX_DEPTH {
                        return Err(UdfError::Malformed("dossiers trop profonds"));
                    }
                    tree.directories.push(child.clone());
                    pending.push((icb, child));
                } else {
                    let entry = self.read_entry(icb)?;
                    tree.files.push(File {
                        path: child,
                        size: entry.size,
                        content: entry.content,
                    });
                }
            }
        }
        tree.files.sort_by(|a, b| a.path.cmp(&b.path));
        tree.directories.sort();
        Ok(tree)
    }
}

/// Lit `buf.len()` octets d'un fichier, depuis `offset`, à travers ses morceaux.
///
/// # Errors
/// Lecture impossible, ou au-delà de la fin des morceaux.
pub fn read_content<R: ReadAt + ?Sized>(
    reader: &R,
    content: &Content,
    offset: u64,
    buf: &mut [u8],
) -> io::Result<()> {
    match content {
        Content::Inline(bytes) => bytes.as_slice().read_exact_at(offset, buf),
        Content::Extents(extents) => {
            let mut skip = offset;
            let mut done = 0;
            for extent in extents {
                if skip >= extent.len {
                    skip -= extent.len;
                    continue;
                }
                let available = extent.len - skip;
                let take = available.min((buf.len() - done) as u64) as usize;
                reader.read_exact_at(extent.offset + skip, &mut buf[done..done + take])?;
                done += take;
                skip = 0;
                if done == buf.len() {
                    return Ok(());
                }
            }
            if done == buf.len() {
                Ok(())
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            }
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    //! Construit une petite image UDF : seuls les champs que le lecteur regarde sont remplis.
    use super::*;

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

    /// Image dont les fichiers sont donnés par chemin, chacun d'un seul morceau.
    #[must_use]
    pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
        const P: usize = 300;
        const BLOCK: usize = 2048;
        use std::collections::BTreeMap;
        let mut dirs: BTreeMap<String, Vec<(String, Option<usize>)>> = BTreeMap::new();
        dirs.insert(String::new(), Vec::new());
        for (index, (path, _)) in files.iter().enumerate() {
            let parts: Vec<&str> = path.split('/').collect();
            for depth in 0..parts.len() {
                let here = parts[..depth].join("/");
                let entry = if depth + 1 == parts.len() {
                    (parts[depth].to_owned(), Some(index))
                } else {
                    (parts[depth].to_owned(), None)
                };
                let list = dirs.entry(here).or_default();
                if !list.contains(&entry) {
                    list.push(entry);
                }
            }
        }
        // Bloc 0 : jeu de fichiers ; puis, pour chaque dossier, son entrée et ses données ; puis les fichiers.
        let mut next = 1;
        let mut dir_blocks = BTreeMap::new();
        for (path, list) in &dirs {
            let bytes = (list.len() + 1) * 64 * 8;
            dir_blocks.insert(path.clone(), next);
            next += 1 + bytes.div_ceil(BLOCK);
        }
        let mut file_blocks = Vec::new();
        for (_, data) in files {
            file_blocks.push(next);
            next += 1 + data.len().div_ceil(BLOCK).max(1);
        }
        let mut img = vec![0u8; (P + next + 1) * BLOCK];
        let block = |b: usize| (P + b) * BLOCK;
        // Reconnaissance du volume : ce que l'analyse d'une image regarde pour dire « ISO » et « UDF ».
        for (sector, id, kind) in [(16, b"CD001", 1u8), (17, b"BEA01", 0), (18, b"NSR02", 0)] {
            let s = sector * BLOCK;
            img[s] = kind;
            img[s + 1..s + 6].copy_from_slice(id);
            img[s + 6] = 1;
        }
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
        let root = dir_blocks[""] as u32;
        let s = block(0);
        img[s + 404..s + 408].copy_from_slice(&root.to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_FILE_SET);
        for (path, list) in &dirs {
            let entry = dir_blocks[path];
            let mut records = vec![fid(FID_PARENT | FID_DIRECTORY, "", entry as u32)];
            for (name, file) in list {
                records.push(match file {
                    Some(index) => fid(0, name, file_blocks[*index] as u32),
                    None => {
                        let child = if path.is_empty() {
                            name.clone()
                        } else {
                            format!("{path}/{name}")
                        };
                        fid(FID_DIRECTORY, name, dir_blocks[&child] as u32)
                    }
                });
            }
            let bytes = records.concat();
            let data = entry + 1;
            img[block(data)..block(data) + bytes.len()].copy_from_slice(&bytes);
            let s = block(entry);
            img[s + 27] = ICB_DIRECTORY;
            img[s + 56..s + 64].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
            img[s + 172..s + 176].copy_from_slice(&8u32.to_le_bytes());
            img[s + 176..s + 180].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            img[s + 180..s + 184].copy_from_slice(&(data as u32).to_le_bytes());
            tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
        }
        for ((_, data), first) in files.iter().zip(&file_blocks) {
            img[block(first + 1)..block(first + 1) + data.len()].copy_from_slice(data);
            let s = block(*first);
            img[s + 56..s + 64].copy_from_slice(&(data.len() as u64).to_le_bytes());
            img[s + 172..s + 176].copy_from_slice(&8u32.to_le_bytes());
            img[s + 176..s + 180].copy_from_slice(&(data.len() as u32).to_le_bytes());
            img[s + 180..s + 184].copy_from_slice(&((first + 1) as u32).to_le_bytes());
            tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
        }
        img
    }

    /// `/SOURCES/boot.wim` en deux morceaux, `/readme.txt` dans son entrée, `/vide/` sans contenu.
    #[must_use]
    pub fn image() -> Vec<u8> {
        const P: usize = 300;
        let mut img = vec![0u8; 700 * 2048];
        let at = |sector: usize| sector * 2048;
        let block = |b: usize| (P + b) * 2048;
        // Ancre : séquence de descripteurs au secteur 257.
        let s = at(256);
        img[s + 16..s + 20].copy_from_slice(&(16 * 2048u32).to_le_bytes());
        img[s + 20..s + 24].copy_from_slice(&257u32.to_le_bytes());
        tag(&mut img[s..s + 2048], TAG_ANCHOR);
        let s = at(257);
        img[s + 188..s + 192].copy_from_slice(&(P as u32).to_le_bytes());
        tag(&mut img[s..s + 2048], TAG_PARTITION);
        let s = at(258);
        img[s + 212..s + 216].copy_from_slice(&2048u32.to_le_bytes());
        img[s + 252..s + 256].copy_from_slice(&0u32.to_le_bytes());
        img[s + 268..s + 272].copy_from_slice(&1u32.to_le_bytes());
        img[s + 440] = 1;
        tag(&mut img[s..s + 2048], TAG_LOGICAL_VOLUME);
        tag(&mut img[at(259)..at(260)], TAG_TERMINATOR);
        // Jeu de fichiers au bloc 0, racine au bloc 1.
        let s = block(0);
        img[s + 404..s + 408].copy_from_slice(&1u32.to_le_bytes());
        tag(&mut img[s..s + 2048], TAG_FILE_SET);
        // Entrée de dossier en ligne ou par morceau : `dir(bloc de l'entrée, bloc des données, octets)`.
        let dir = |img: &mut Vec<u8>, entry: usize, data: usize, records: &[Vec<u8>]| {
            let bytes: Vec<u8> = records.concat();
            img[block(data)..block(data) + bytes.len()].copy_from_slice(&bytes);
            let s = block(entry);
            img[s + 27] = ICB_DIRECTORY;
            img[s + 56..s + 64].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
            img[s + 172..s + 176].copy_from_slice(&8u32.to_le_bytes());
            img[s + 176..s + 180].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            img[s + 180..s + 184].copy_from_slice(&(data as u32).to_le_bytes());
            tag(&mut img[s..s + 2048], TAG_FILE_ENTRY);
        };
        dir(
            &mut img,
            1,
            2,
            &[
                fid(FID_PARENT | FID_DIRECTORY, "", 1),
                fid(FID_DIRECTORY, "SOURCES", 3),
                fid(0, "readme.txt", 5),
                fid(FID_DIRECTORY, "vide", 6),
                fid(FID_DELETED, "efface.txt", 5),
            ],
        );
        dir(
            &mut img,
            3,
            4,
            &[
                fid(FID_PARENT | FID_DIRECTORY, "", 1),
                fid(0, "boot.wim", 8),
            ],
        );
        // Fichier en ligne.
        let s = block(5);
        img[s + 34] = 3;
        img[s + 56..s + 64].copy_from_slice(&5u64.to_le_bytes());
        img[s + 172..s + 176].copy_from_slice(&5u32.to_le_bytes());
        img[s + 176..s + 181].copy_from_slice(b"salut");
        tag(&mut img[s..s + 2048], TAG_FILE_ENTRY);
        dir(&mut img, 6, 7, &[fid(FID_PARENT | FID_DIRECTORY, "", 1)]);
        // Fichier de 3000 octets, en deux morceaux séparés, par une entrée étendue.
        let s = block(8);
        img[s + 56..s + 64].copy_from_slice(&3000u64.to_le_bytes());
        img[s + 212..s + 216].copy_from_slice(&16u32.to_le_bytes());
        img[s + 216..s + 220].copy_from_slice(&2048u32.to_le_bytes());
        img[s + 220..s + 224].copy_from_slice(&20u32.to_le_bytes());
        img[s + 224..s + 228].copy_from_slice(&952u32.to_le_bytes());
        img[s + 228..s + 232].copy_from_slice(&30u32.to_le_bytes());
        tag(&mut img[s..s + 2048], TAG_EXTENDED_FILE_ENTRY);
        img[block(20)..block(20) + 2048].fill(0xAA);
        img[block(30)..block(30) + 952].fill(0xBB);
        img
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn names(tree: &Tree) -> Vec<String> {
        tree.files.iter().map(|f| f.path.join("/")).collect()
    }

    #[test]
    fn l_arbre_se_lit_avec_ses_trois_formes_de_contenu() {
        let image = testing::image();
        let udf = Udf::open(image.as_slice()).unwrap();
        let tree = udf.tree().unwrap();
        assert_eq!(names(&tree), ["SOURCES/boot.wim", "readme.txt"]);
        assert_eq!(
            tree.directories,
            [vec!["SOURCES".to_owned()], vec!["vide".to_owned()]]
        );

        let inline = tree.find(&["README.TXT"]).unwrap();
        assert_eq!(inline.size, 5);
        let mut buf = [0; 5];
        read_content(image.as_slice(), &inline.content, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"salut");

        let parts = tree.find(&["sources", "boot.wim"]).unwrap();
        assert_eq!(parts.size, 3000);
        let mut buf = vec![0; 3000];
        read_content(image.as_slice(), &parts.content, 0, &mut buf).unwrap();
        assert!(buf[..2048].iter().all(|b| *b == 0xAA));
        assert!(buf[2048..].iter().all(|b| *b == 0xBB));
        // À cheval sur les deux morceaux.
        let mut buf = [0; 4];
        read_content(image.as_slice(), &parts.content, 2046, &mut buf).unwrap();
        assert_eq!(buf, [0xAA, 0xAA, 0xBB, 0xBB]);
    }

    #[test]
    fn sans_ancre_ce_n_est_pas_de_l_udf() {
        let image = vec![0u8; 1 << 20];
        assert!(matches!(Udf::open(image.as_slice()), Err(UdfError::NotUdf)));
        assert!(matches!(Udf::open([].as_slice()), Err(UdfError::NotUdf)));
    }

    #[test]
    fn un_nom_ne_peut_pas_sortir_de_l_arbre() {
        for raw in [
            &b"\x08.."[..],
            b"\x08a/b",
            b"\x08a\\b",
            b"\x08",
            b"",
            b"\x07abc",
            b"\x10abc",
        ] {
            assert!(decode_name(raw).is_err(), "{raw:?}");
        }
        assert_eq!(decode_name(b"\x08caf\xE9").unwrap(), "café");
        assert_eq!(decode_name(b"\x10\x00a\x00b").unwrap(), "ab");
    }

    /// Chaque octet de la structure abîmé tour à tour : jamais de panique ni de boucle sans fin.
    #[test]
    fn une_image_abimee_ne_fait_jamais_paniquer() {
        let good = testing::image();
        let ranges = [
            256 * 2048..256 * 2048 + 32,
            258 * 2048 + 200..258 * 2048 + 460,
            300 * 2048..309 * 2048 + 240,
        ];
        for range in ranges {
            for at in range.step_by(3) {
                let mut bad = good.clone();
                bad[at] ^= 0xFF;
                if let Ok(udf) = Udf::open(bad.as_slice()) {
                    let _ = udf.tree();
                }
            }
        }
    }
}
