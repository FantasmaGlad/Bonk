//! Découpage d'un fichier WIM en morceaux `.swm`, sans recompression : FAT32 ne garde pas un fichier de
//! 4 Gio. Chaque morceau a son en-tête et sa table ; les ressources compressées sont recopiées telles
//! quelles, et une ressource n'est jamais répartie entre deux morceaux (format WIM).

use std::fmt;

const HEADER_LEN: usize = 208;
const MAGIC: &[u8; 8] = b"MSWIM\0\0\0";
const ENTRY_LEN: usize = 50;
const RESOURCE_METADATA: u8 = 0x02;
/// Plusieurs fichiers compressés ensemble (LZMS, `install.esd`) : découper imposerait de recompresser.
const RESOURCE_SOLID: u8 = 0x10;
const FLAG_SPANNED: u32 = 0x08;

const AT_FLAGS: usize = 16;
const AT_PART: usize = 40;
const AT_TOTAL_PARTS: usize = 42;
const AT_LOOKUP: usize = 48;
const AT_XML: usize = 72;
const AT_BOOT: usize = 96;
const AT_INTEGRITY: usize = 124;
const RESOURCE_LEN: usize = 24;

#[derive(Debug, PartialEq, Eq)]
pub enum WimError {
    NotWim,
    Malformed(&'static str),
    /// Ressources solides : aucun découpage sans recompression.
    Solid,
    /// Une ressource seule dépasse la taille permise pour un morceau.
    ResourceTooLarge(u64),
}

impl fmt::Display for WimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWim => f.write_str("not a WIM file"),
            Self::Malformed(what) => write!(f, "WIM unreadable: {what}"),
            Self::Solid => f.write_str(
                "WIM with solid resources (install.esd): cannot be split without recompression",
            ),
            Self::ResourceTooLarge(len) => {
                write!(f, "a resource of {len} bytes fits in no part")
            }
        }
    }
}

impl std::error::Error for WimError {}

/// Un morceau d'un fichier produit : des octets calculés, ou une plage du WIM d'origine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Bytes(Vec<u8>),
    Source { offset: u64, len: u64 },
}

impl Segment {
    #[must_use]
    pub fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Source { len, .. } => *len,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Emplacement d'une ressource, tel que l'écrit le format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Resource {
    stored: u64,
    flags: u8,
    offset: u64,
    original: u64,
}

impl Resource {
    fn read(bytes: &[u8]) -> Self {
        let word = |at: usize| {
            let mut field = [0; 8];
            field.copy_from_slice(&bytes[at..at + 8]);
            u64::from_le_bytes(field)
        };
        let first = word(0);
        Self {
            stored: first & 0x00FF_FFFF_FFFF_FFFF,
            flags: bytes[7],
            offset: word(8),
            original: word(16),
        }
    }

    fn encode(self) -> [u8; RESOURCE_LEN] {
        let mut out = [0; RESOURCE_LEN];
        out[..8].copy_from_slice(&self.stored.to_le_bytes());
        out[7] = self.flags;
        out[8..16].copy_from_slice(&self.offset.to_le_bytes());
        out[16..24].copy_from_slice(&self.original.to_le_bytes());
        out
    }

    const NONE: Self = Self {
        stored: 0,
        flags: 0,
        offset: 0,
        original: 0,
    };
}

/// En-tête lu : où lire la table des ressources et le XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub lookup_offset: u64,
    pub lookup_len: u64,
}

/// # Errors
/// [`WimError::NotWim`] sans signature ; une table compressée ou un WIM déjà découpé sont refusés.
pub fn read_header(header: &[u8]) -> Result<Header, WimError> {
    if header.len() < HEADER_LEN || &header[..8] != MAGIC {
        return Err(WimError::NotWim);
    }
    let total = u16::from_le_bytes([header[AT_TOTAL_PARTS], header[AT_TOTAL_PARTS + 1]]);
    if total > 1 {
        return Err(WimError::Malformed("WIM already split"));
    }
    let lookup = Resource::read(&header[AT_LOOKUP..AT_LOOKUP + RESOURCE_LEN]);
    if lookup.flags & 0x04 != 0 {
        return Err(WimError::Malformed("compressed resource table"));
    }
    Ok(Header {
        lookup_offset: lookup.offset,
        lookup_len: lookup.stored,
    })
}

/// Vrai si ce WIM contient des ressources solides.
///
/// # Errors
/// Table mal formée.
pub fn has_solid_resources(lookup: &[u8]) -> Result<bool, WimError> {
    if !lookup.len().is_multiple_of(ENTRY_LEN) {
        return Err(WimError::Malformed("resource table"));
    }
    Ok(lookup
        .as_chunks::<ENTRY_LEN>()
        .0
        .iter()
        .any(|entry| Resource::read(entry).flags & RESOURCE_SOLID != 0))
}

/// Un morceau en construction : ses ressources, et leur place.
#[derive(Default)]
struct Part {
    entries: Vec<(Resource, [u8; ENTRY_LEN - RESOURCE_LEN])>,
    resources_len: u64,
}

impl Part {
    fn table_len(&self) -> u64 {
        (self.entries.len() * ENTRY_LEN) as u64
    }
}

/// Découpe : chaque élément est la liste des segments d'un fichier, dans l'ordre (`install.swm`,
/// `install2.swm`...). Les métadonnées vont toutes au premier morceau, que l'installateur lit d'abord.
/// Aucun morceau ne dépasse `part_limit` octets.
///
/// # Errors
/// [`WimError::Solid`], [`WimError::ResourceTooLarge`], ou une structure incohérente.
pub fn split(
    header: &[u8],
    lookup: &[u8],
    file_size: u64,
    part_limit: u64,
) -> Result<Vec<Vec<Segment>>, WimError> {
    read_header(header)?;
    if has_solid_resources(lookup)? {
        return Err(WimError::Solid);
    }
    let xml = Resource::read(&header[AT_XML..AT_XML + RESOURCE_LEN]);
    let boot = Resource::read(&header[AT_BOOT..AT_BOOT + RESOURCE_LEN]);
    let fixed = HEADER_LEN as u64 + xml.stored;

    let mut metadata = Vec::new();
    let mut streams = Vec::new();
    for raw in lookup.as_chunks::<ENTRY_LEN>().0 {
        let resource = Resource::read(raw);
        if resource
            .offset
            .checked_add(resource.stored)
            .is_none_or(|end| end > file_size)
        {
            return Err(WimError::Malformed("resource outside the file"));
        }
        let mut rest = [0; ENTRY_LEN - RESOURCE_LEN];
        rest.copy_from_slice(&raw[RESOURCE_LEN..]);
        if resource.flags & RESOURCE_METADATA != 0 {
            metadata.push((resource, rest));
        } else {
            streams.push((resource, rest));
        }
    }
    if xml
        .offset
        .checked_add(xml.stored)
        .is_none_or(|end| end > file_size)
    {
        return Err(WimError::Malformed("XML outside the file"));
    }

    let mut parts = vec![Part::default()];
    for item in metadata.into_iter().chain(streams) {
        let need = |part: &Part| {
            fixed + part.resources_len + item.0.stored + part.table_len() + ENTRY_LEN as u64
        };
        if fixed + item.0.stored + ENTRY_LEN as u64 > part_limit {
            return Err(WimError::ResourceTooLarge(item.0.stored));
        }
        if parts.last().is_some_and(|part| need(part) > part_limit) {
            parts.push(Part::default());
        }
        if let Some(part) = parts.last_mut() {
            part.resources_len += item.0.stored;
            part.entries.push(item);
        }
    }

    let total = u16::try_from(parts.len()).map_err(|_| WimError::Malformed("too many parts"))?;
    let mut files = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let number = u16::try_from(index + 1).map_err(|_| WimError::Malformed("too many parts"))?;
        let mut position = HEADER_LEN as u64;
        let mut sources = Vec::new();
        let mut table = Vec::with_capacity(part.entries.len() * ENTRY_LEN);
        let mut new_boot = Resource::NONE;
        for (resource, rest) in &part.entries {
            let placed = Resource {
                offset: position,
                ..*resource
            };
            if resource.offset == boot.offset
                && boot.stored != 0
                && resource.flags & RESOURCE_METADATA != 0
            {
                new_boot = placed;
            }
            table.extend_from_slice(&placed.encode());
            table.extend_from_slice(rest);
            let at = table.len() - (ENTRY_LEN - RESOURCE_LEN);
            table[at..at + 2].copy_from_slice(&number.to_le_bytes());
            sources.push(Segment::Source {
                offset: resource.offset,
                len: resource.stored,
            });
            position += resource.stored;
        }
        let lookup_resource = Resource {
            stored: table.len() as u64,
            flags: 0,
            offset: position,
            original: table.len() as u64,
        };
        position += table.len() as u64;
        let xml_resource = Resource {
            offset: position,
            ..xml
        };
        let mut head = header[..HEADER_LEN].to_vec();
        let flags = u32::from_le_bytes([
            head[AT_FLAGS],
            head[AT_FLAGS + 1],
            head[AT_FLAGS + 2],
            head[AT_FLAGS + 3],
        ]);
        head[AT_FLAGS..AT_FLAGS + 4].copy_from_slice(&(flags | FLAG_SPANNED).to_le_bytes());
        head[AT_PART..AT_PART + 2].copy_from_slice(&number.to_le_bytes());
        head[AT_TOTAL_PARTS..AT_TOTAL_PARTS + 2].copy_from_slice(&total.to_le_bytes());
        head[AT_LOOKUP..AT_LOOKUP + RESOURCE_LEN].copy_from_slice(&lookup_resource.encode());
        head[AT_XML..AT_XML + RESOURCE_LEN].copy_from_slice(&xml_resource.encode());
        head[AT_BOOT..AT_BOOT + RESOURCE_LEN].copy_from_slice(&new_boot.encode());
        // Les sommes d'intégrité portent sur le fichier d'origine : un morceau n'en a pas.
        head[AT_INTEGRITY..AT_INTEGRITY + RESOURCE_LEN].fill(0);
        let mut segments = vec![Segment::Bytes(head)];
        segments.extend(sources);
        segments.push(Segment::Bytes(table));
        segments.push(Segment::Source {
            offset: xml.offset,
            len: xml.stored,
        });
        files.push(segments);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// WIM minimal : 1 métadonnée puis `streams` ressources de `each` octets, puis le XML.
    fn wim(streams: usize, each: u64, solid: bool) -> (Vec<u8>, Vec<u8>, u64) {
        let mut table = Vec::new();
        let mut position = HEADER_LEN as u64;
        let mut entry = |flags: u8, len: u64, tag: u8, position: &mut u64| {
            let resource = Resource {
                stored: len,
                flags,
                offset: *position,
                original: len,
            };
            table.extend_from_slice(&resource.encode());
            table.extend_from_slice(&1u16.to_le_bytes());
            table.extend_from_slice(&1u32.to_le_bytes());
            table.extend_from_slice(&[tag; 20]);
            *position += len;
        };
        entry(RESOURCE_METADATA, 100, 0xAA, &mut position);
        for n in 0..streams {
            entry(
                if solid { RESOURCE_SOLID } else { 0 },
                each,
                n as u8,
                &mut position,
            );
        }
        let mut header = vec![0; HEADER_LEN];
        header[..8].copy_from_slice(MAGIC);
        header[AT_PART] = 1;
        header[AT_TOTAL_PARTS] = 1;
        let lookup = Resource {
            stored: table.len() as u64,
            flags: 0,
            offset: position,
            original: table.len() as u64,
        };
        header[AT_LOOKUP..AT_LOOKUP + RESOURCE_LEN].copy_from_slice(&lookup.encode());
        let xml = Resource {
            stored: 50,
            flags: 0,
            offset: position + table.len() as u64,
            original: 50,
        };
        header[AT_XML..AT_XML + RESOURCE_LEN].copy_from_slice(&xml.encode());
        let meta = Resource {
            stored: 100,
            flags: RESOURCE_METADATA,
            offset: HEADER_LEN as u64,
            original: 100,
        };
        header[AT_BOOT..AT_BOOT + RESOURCE_LEN].copy_from_slice(&meta.encode());
        header[AT_INTEGRITY] = 9;
        let size = xml.offset + 50;
        (header, table, size)
    }

    fn total(segments: &[Segment]) -> u64 {
        segments.iter().map(Segment::len).sum()
    }

    fn sources(segments: &[Segment]) -> Vec<(u64, u64)> {
        segments
            .iter()
            .filter_map(|s| match s {
                Segment::Source { offset, len } => Some((*offset, *len)),
                Segment::Bytes(_) => None,
            })
            .collect()
    }

    #[test]
    fn chaque_morceau_a_ses_ressources_sa_table_et_son_en_tete() {
        let (header, table, size) = wim(5, 1000, false);
        let limit = 2500;
        let parts = split(&header, &table, size, limit).unwrap();
        assert_eq!(parts.len(), 3);
        let mut seen = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            assert!(total(part) <= limit, "{}", total(part));
            let Segment::Bytes(head) = &part[0] else {
                panic!()
            };
            assert_eq!(head.len(), HEADER_LEN);
            assert_eq!(
                u16::from_le_bytes([head[AT_PART], head[AT_PART + 1]]),
                index as u16 + 1
            );
            assert_eq!(head[AT_TOTAL_PARTS], 3);
            assert_eq!(head[AT_FLAGS] & FLAG_SPANNED as u8, FLAG_SPANNED as u8);
            assert_eq!(head[AT_INTEGRITY], 0);
            let lookup = Resource::read(&head[AT_LOOKUP..AT_LOOKUP + RESOURCE_LEN]);
            let Segment::Bytes(table) = &part[part.len() - 2] else {
                panic!()
            };
            assert_eq!(lookup.stored, table.len() as u64);
            // Dans sa table, chaque ressource est à sa nouvelle place, avec le numéro du morceau.
            let mut position = HEADER_LEN as u64;
            for entry in table.as_chunks::<ENTRY_LEN>().0 {
                let resource = Resource::read(entry);
                assert_eq!(resource.offset, position);
                assert_eq!(u16::from_le_bytes([entry[24], entry[25]]), index as u16 + 1);
                position += resource.stored;
            }
            assert_eq!(lookup.offset, position);
            let xml = Resource::read(&head[AT_XML..AT_XML + RESOURCE_LEN]);
            assert_eq!(xml.offset, position + lookup.stored);
            assert_eq!(xml.offset + xml.stored, total(part));
            seen.extend(sources(part));
        }
        // Rien de perdu ni de doublé : toutes les ressources, plus une fois le XML par morceau.
        let (xml_offset, xml_len) = (size - 50, 50);
        let resources: Vec<_> = seen.iter().filter(|s| s.0 != xml_offset).collect();
        assert_eq!(resources.len(), 6);
        assert_eq!(
            seen.iter().filter(|s| **s == (xml_offset, xml_len)).count(),
            3
        );
        // Les métadonnées et le démarrage sont au premier morceau.
        let Segment::Bytes(first) = &parts[0][0] else {
            panic!()
        };
        let boot = Resource::read(&first[AT_BOOT..AT_BOOT + RESOURCE_LEN]);
        assert_eq!(boot.offset, HEADER_LEN as u64);
        let Segment::Bytes(second) = &parts[1][0] else {
            panic!()
        };
        assert_eq!(
            Resource::read(&second[AT_BOOT..AT_BOOT + RESOURCE_LEN]),
            Resource::NONE
        );
    }

    #[test]
    fn les_refus_sont_dits() {
        let (header, table, size) = wim(2, 1000, true);
        assert_eq!(split(&header, &table, size, 1 << 20), Err(WimError::Solid));
        let (header, table, size) = wim(1, 10_000, false);
        assert_eq!(
            split(&header, &table, size, 2000),
            Err(WimError::ResourceTooLarge(10_000))
        );
        assert_eq!(read_header(&[0; 208]), Err(WimError::NotWim));
        assert_eq!(read_header(b"MSWIM"), Err(WimError::NotWim));
        let (mut header, table, size) = wim(1, 10, false);
        header[AT_TOTAL_PARTS] = 2;
        assert!(split(&header, &table, size, 1 << 20).is_err());
        let (header, table, _) = wim(1, 10, false);
        assert!(split(&header, &table, 5, 1 << 20).is_err());
    }
}
