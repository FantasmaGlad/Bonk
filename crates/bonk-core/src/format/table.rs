//! Écriture d'une table MBR ou GPT à une seule partition, alignée sur 1 Mio.

use schemars::JsonSchema;
use serde::Serialize;

use super::{ALIGNMENT, Filesystem, FormatError, Ids, zero_range};
use crate::block::BlockDevice;
use crate::partition::crc32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Gpt,
    Mbr,
}

impl TableKind {
    pub const ALL: [Self; 2] = [Self::Gpt, Self::Mbr];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Gpt => "gpt",
            Self::Mbr => "mbr",
        }
    }
}

/// La partition que portera le support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PartitionPlan {
    pub first_lba: u64,
    pub last_lba: u64,
}

impl PartitionPlan {
    #[must_use]
    pub fn sectors(self) -> u64 {
        self.last_lba - self.first_lba + 1
    }
}

const GPT_ENTRIES: u32 = 128;
const GPT_ENTRY_SIZE: u32 = 128;
const GPT_REVISION: u32 = 0x0001_0000;
const GPT_HEADER_SIZE: u32 = 92;
const GPT_ENTRIES_LBA: u64 = 2;
const MBR_PROTECTIVE: u8 = 0xEE;
/// « Basic data » : FAT, exFAT et NTFS, EBD0A0A2-B9E5-4433-87C0-68B6B72699C7.
const GUID_BASIC_DATA: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];
/// « Linux filesystem », 0FC63DAF-8483-4772-8E79-3D69D8477DE4.
const GUID_LINUX: [u8; 16] = [
    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4,
];

fn gpt_type(fs: Filesystem) -> [u8; 16] {
    match fs {
        Filesystem::Fat32 | Filesystem::Exfat | Filesystem::Ntfs => GUID_BASIC_DATA,
        Filesystem::Ext4 => GUID_LINUX,
    }
}

fn mbr_type(fs: Filesystem) -> u8 {
    match fs {
        Filesystem::Fat32 => 0x0C,
        Filesystem::Exfat | Filesystem::Ntfs => 0x07,
        Filesystem::Ext4 => 0x83,
    }
}

fn gpt_table_sectors(block: u64) -> u64 {
    (u64::from(GPT_ENTRIES) * u64::from(GPT_ENTRY_SIZE)).div_ceil(block)
}

/// Où sera la partition, sans rien écrire : sert aussi à `--dry-run`, et à refuser avant le premier
/// geste. Un MBR n'adresse pas au-delà de 2^32 secteurs.
pub fn plan(size: u64, block: u32, kind: TableKind) -> Result<PartitionPlan, FormatError> {
    let block = u64::from(block);
    let sectors = size / block;
    let first_lba = ALIGNMENT.div_ceil(block);
    let last = match kind {
        TableKind::Gpt => sectors.checked_sub(gpt_table_sectors(block) + 2),
        TableKind::Mbr => {
            if sectors > u64::from(u32::MAX) {
                return Err(FormatError::TooLarge(format!(
                    "une table MBR n'adresse pas {size} octets : choisir GPT"
                )));
            }
            sectors.checked_sub(1)
        }
    };
    match last {
        Some(last_lba) if last_lba > first_lba => Ok(PartitionPlan {
            first_lba,
            last_lba,
        }),
        _ => Err(FormatError::TooSmall(format!(
            "support de {size} octets trop petit pour une table et une partition"
        ))),
    }
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// Un secteur 0 : la signature et une seule entrée, sans code d'amorçage (support de stockage).
fn mbr_sector(block: usize, signature: u32, kind: u8, first: u32, sectors: u32) -> Vec<u8> {
    let mut sector = vec![0u8; block];
    put_u32(&mut sector, 440, signature);
    // CHS hors de portée : seules les valeurs LBA comptent.
    sector[446 + 1..446 + 4].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    sector[446 + 4] = kind;
    sector[446 + 5..446 + 8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    put_u32(&mut sector, 446 + 8, first);
    put_u32(&mut sector, 446 + 12, sectors);
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

struct GptHeader {
    my_lba: u64,
    alternate_lba: u64,
    last_usable: u64,
    entries_lba: u64,
    first_usable: u64,
    table_crc: u32,
    guid: [u8; 16],
}

fn gpt_header(block: usize, h: &GptHeader) -> Vec<u8> {
    let mut s = vec![0u8; block];
    s[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut s, 8, GPT_REVISION);
    put_u32(&mut s, 12, GPT_HEADER_SIZE);
    put_u64(&mut s, 24, h.my_lba);
    put_u64(&mut s, 32, h.alternate_lba);
    put_u64(&mut s, 40, h.first_usable);
    put_u64(&mut s, 48, h.last_usable);
    s[56..72].copy_from_slice(&h.guid);
    put_u64(&mut s, 72, h.entries_lba);
    put_u32(&mut s, 80, GPT_ENTRIES);
    put_u32(&mut s, 84, GPT_ENTRY_SIZE);
    put_u32(&mut s, 88, h.table_crc);
    let crc = crc32(&s[..GPT_HEADER_SIZE as usize]);
    put_u32(&mut s, 16, crc);
    s
}

fn write_gpt(
    dev: &mut dyn BlockDevice,
    fs: Filesystem,
    part: PartitionPlan,
    label: Option<&str>,
    ids: &Ids,
) -> Result<(), FormatError> {
    let block = u64::from(dev.block_size());
    let block_len = dev.block_size() as usize;
    let last = dev.size() / block - 1;
    let table_sectors = gpt_table_sectors(block);

    let mut table = vec![0u8; usize::try_from(table_sectors * block).unwrap_or(0)];
    table[..16].copy_from_slice(&gpt_type(fs));
    table[16..32].copy_from_slice(&ids.partition_guid);
    put_u64(&mut table, 32, part.first_lba);
    put_u64(&mut table, 40, part.last_lba);
    for (i, unit) in label
        .unwrap_or_default()
        .encode_utf16()
        .take(36)
        .enumerate()
    {
        table[56 + i * 2..58 + i * 2].copy_from_slice(&unit.to_le_bytes());
    }
    let table_len = (GPT_ENTRIES * GPT_ENTRY_SIZE) as usize;
    let table_crc = crc32(&table[..table_len]);

    let backup_entries = last - table_sectors;
    let header = |my_lba, alternate_lba, entries_lba| GptHeader {
        my_lba,
        alternate_lba,
        last_usable: backup_entries - 1,
        entries_lba,
        first_usable: GPT_ENTRIES_LBA + table_sectors,
        table_crc,
        guid: ids.disk_guid,
    };
    // Le secours d'abord : une coupure laisse un support sans table principale valide, jamais une table
    // principale qui pointe vers un secours absent.
    dev.write_at(backup_entries * block, &table)?;
    dev.write_at(
        last * block,
        &gpt_header(block_len, &header(last, 1, backup_entries)),
    )?;
    dev.write_at(GPT_ENTRIES_LBA * block, &table)?;
    dev.write_at(
        block,
        &gpt_header(block_len, &header(1, last, GPT_ENTRIES_LBA)),
    )?;
    let protective = u32::try_from(last).unwrap_or(u32::MAX);
    dev.write_at(
        0,
        &mbr_sector(block_len, ids.disk_signature, MBR_PROTECTIVE, 1, protective),
    )?;
    Ok(())
}

/// Efface les anciennes tables et le début de la nouvelle partition, puis écrit la table ; une
/// géométrie refusée n'écrit rien. Le vidage reste à l'appelant, une fois le système de fichiers posé.
pub fn write_table(
    dev: &mut dyn BlockDevice,
    kind: TableKind,
    fs: Filesystem,
    label: Option<&str>,
    ids: &Ids,
) -> Result<PartitionPlan, FormatError> {
    let part = plan(dev.size(), dev.block_size(), kind)?;
    let block = u64::from(dev.block_size());
    // Un ancien GPT de secours survivrait en fin de support, une ancienne signature au début de la
    // partition : le système reconnaîtrait ce qui n'existe plus.
    let wiped = (part.first_lba * block + ALIGNMENT).min(dev.size());
    zero_range(dev, 0, wiped)?;
    let size = dev.size();
    let tail = ALIGNMENT.min(size - wiped);
    zero_range(dev, size - tail, tail)?;
    match kind {
        TableKind::Gpt => write_gpt(dev, fs, part, label, ids)?,
        TableKind::Mbr => {
            let first = u32::try_from(part.first_lba).unwrap_or(u32::MAX);
            let sectors = u32::try_from(part.sectors()).unwrap_or(u32::MAX);
            let sector = mbr_sector(
                usize::try_from(block).unwrap_or(512),
                ids.disk_signature,
                mbr_type(fs),
                first,
                sectors,
            );
            dev.write_at(0, &sector)?;
        }
    }
    Ok(part)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;
    use crate::partition::{Severity, inspect};

    const MIB: u64 = 1 << 20;

    fn ids() -> Ids {
        Ids::from_seed(b"test")
    }

    fn no_error(dev: &MemoryDevice) -> crate::partition::Layout {
        let layout = inspect(dev).unwrap_or_else(|e| panic!("{e}"));
        let graves: Vec<_> = layout
            .findings
            .iter()
            .filter(|f| f.severity != Severity::Info)
            .collect();
        assert!(graves.is_empty(), "{graves:?}");
        layout
    }

    #[test]
    fn gpt_relu_sans_constat_et_partition_alignee() -> Result<(), Box<dyn std::error::Error>> {
        for block in [512u32, 4096] {
            let mut dev = MemoryDevice::new(64 * MIB, block)?;
            let part = write_table(
                &mut dev,
                TableKind::Gpt,
                Filesystem::Fat32,
                Some("CLE"),
                &ids(),
            )?;
            let layout = no_error(&dev);
            let gpt = layout.gpt.as_ref().ok_or("pas de GPT")?;
            assert_eq!(gpt.entries.len(), 1);
            assert_eq!(gpt.entries[0].first_lba, part.first_lba);
            assert_eq!(gpt.entries[0].last_lba, part.last_lba);
            assert_eq!(gpt.entries[0].name, "CLE");
            assert_eq!(part.first_lba * u64::from(block), MIB);
            assert!(gpt.backup.is_some());
        }
        Ok(())
    }

    #[test]
    fn mbr_relu_sans_constat() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(64 * MIB, 512)?;
        let part = write_table(&mut dev, TableKind::Mbr, Filesystem::Exfat, None, &ids())?;
        let layout = no_error(&dev);
        assert!(layout.gpt.is_none());
        let mbr = layout.mbr.as_ref().ok_or("pas de MBR")?;
        assert_eq!(mbr.entries.len(), 1);
        assert_eq!(mbr.entries[0].kind, 0x07);
        assert_eq!(u64::from(mbr.entries[0].start_lba), part.first_lba);
        assert_eq!(part.last_lba, 64 * MIB / 512 - 1);
        Ok(())
    }

    #[test]
    fn passer_de_gpt_a_mbr_ne_laisse_aucun_secours() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(64 * MIB, 512)?;
        write_table(&mut dev, TableKind::Gpt, Filesystem::Fat32, None, &ids())?;
        write_table(&mut dev, TableKind::Mbr, Filesystem::Fat32, None, &ids())?;
        assert!(no_error(&dev).gpt.is_none());
        let end = &dev.as_bytes()[usize::try_from(63 * MIB)?..];
        assert!(end.iter().all(|b| *b == 0));
        Ok(())
    }

    #[test]
    fn une_ancienne_signature_au_debut_de_la_partition_est_effacee()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::from_bytes(vec![0xAB; usize::try_from(8 * MIB)?], 512)?;
        write_table(&mut dev, TableKind::Mbr, Filesystem::Ext4, None, &ids())?;
        let begin = usize::try_from(MIB)?;
        assert!(dev.as_bytes()[begin..begin * 2].iter().all(|b| *b == 0));
        Ok(())
    }

    #[test]
    fn plan_refuse_trop_petit_et_mbr_trop_grand() {
        assert!(matches!(
            plan(MIB, 512, TableKind::Gpt),
            Err(FormatError::TooSmall(_))
        ));
        assert!(matches!(
            plan(4 << 40, 512, TableKind::Mbr),
            Err(FormatError::TooLarge(_))
        ));
        assert!(plan(4 << 40, 512, TableKind::Gpt).is_ok());
    }

    #[test]
    fn meme_graine_memes_octets() -> Result<(), Box<dyn std::error::Error>> {
        let mut a = MemoryDevice::new(8 * MIB, 512)?;
        let mut b = MemoryDevice::new(8 * MIB, 512)?;
        write_table(&mut a, TableKind::Gpt, Filesystem::Fat32, None, &ids())?;
        write_table(&mut b, TableKind::Gpt, Filesystem::Fat32, None, &ids())?;
        assert!(a.as_bytes() == b.as_bytes());
        Ok(())
    }
}
