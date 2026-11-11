use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::block::BlockDevice;
use crate::format::exfat_read::{FileEntry, Reader};
use crate::format::table::{
    self, GUID_ESP, MBR_ESP, PartitionPlan, Slot, TableKind, gpt_type, mbr_type, usable_end,
};
use crate::format::{ALIGNMENT, Filesystem, FormatError, Ids, exfat, fat32, guid};
use crate::format::{NewFile, fat32_tree};
use crate::partition::{MbrEntry, Severity, inspect};

/// exFAT volume label and partition name of the data partition.
pub const VAULT_LABEL: &str = "BonkVault";
/// GPT partition name of the boot partition.
pub const EFI_NAME: &str = "BonkEFI";
/// FAT32 volume label of the boot partition: upper-case ASCII.
pub const EFI_LABEL: &str = "BONKEFI";
const EFI_MIN_BYTES: u64 = 64 << 20;
const EFI_MAX_BYTES: u64 = 1 << 30;
const VAULT_MIN_BYTES: u64 = 64 << 20;
const ESP_GUID_TEXT: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
/// Menu read by GRUB: lists the ISO files of the data partition at boot, so copying one needs no tool.
pub const MENU: &str = include_str!("vault_menu.cfg");
/// The path the GRUB image searches for.
pub const MENU_PATH: &str = "bonk/grub.cfg";
/// Ubuntu's iso-scan step with exFAT accepted, put into its initrd by the menu: kept apart, as it follows
/// casper's own script.
pub const CASPER_EXFAT: &str = include_str!("vault_casper_exfat.sh");
/// Next to the menu, which keeps GRUB's `config_directory` to name it.
pub const CASPER_EXFAT_PATH: &str = "bonk/casper/20iso_scan";

/// The live systems the menu starts from an ISO file, recognised by the same files it looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IsoFamily {
    DebianLive,
    Ubuntu,
    Arch,
    FedoraLive,
}

impl IsoFamily {
    /// Files the menu tests, in its order: the first family whose files are all there wins.
    const RULES: [(Self, &'static [&'static str]); 4] = [
        (Self::DebianLive, &["live/vmlinuz", "live/initrd.img"]),
        (Self::Ubuntu, &["casper/vmlinuz", "casper/initrd"]),
        (
            Self::Arch,
            &[
                "arch/boot/x86_64/vmlinuz-linux",
                "arch/boot/x86_64/initramfs-linux.img",
            ],
        ),
        (
            Self::FedoraLive,
            &["boot/x86_64/loader/linux", "boot/x86_64/loader/initrd"],
        ),
    ];
}

/// The family the vault menu would start, `None` for an ISO it shows as not supported.
///
/// # Errors
/// Not an ISO 9660 image, or unreadable.
pub fn iso_family<R: std::io::Read + std::io::Seek>(
    iso: &mut crate::iso9660::Iso<R>,
) -> std::io::Result<Option<IsoFamily>> {
    for (family, files) in IsoFamily::RULES {
        let mut all = true;
        for file in files {
            all &= iso.file_size(file)?.is_some();
        }
        if all {
            // The menu checks the label too: other Kiwi images share this layout.
            let fedora = family != IsoFamily::FedoraLive || iso.label().starts_with("Fedora-");
            return Ok(fedora.then_some(family));
        }
    }
    Ok(None)
}

/// A file of the boot partition: path from its root, `/` separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// Removable-media name of an architecture's EFI image: firmware loads the one of its own processor.
#[must_use]
pub fn efi_file_name(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("BOOTX64.EFI"),
        "aarch64" => Some("BOOTAA64.EFI"),
        "riscv64" => Some("BOOTRISCV64.EFI"),
        _ => None,
    }
}

/// GRUB images by architecture, then the menu and the files it uses.
///
/// # Errors
/// [`FormatError::Unsupported`] for an architecture without an EFI removable-media name.
pub fn boot_files(efi: &[(&str, Vec<u8>)]) -> Result<Vec<BootFile>, FormatError> {
    let mut files = efi
        .iter()
        .map(|(arch, data)| {
            let name = efi_file_name(arch)
                .ok_or_else(|| FormatError::Unsupported(format!("no EFI boot file for {arch}")))?;
            Ok(BootFile {
                path: format!("EFI/BOOT/{name}"),
                data: data.clone(),
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    files.push(BootFile {
        path: MENU_PATH.to_owned(),
        data: MENU.as_bytes().to_vec(),
    });
    files.push(BootFile {
        path: CASPER_EXFAT_PATH.to_owned(),
        data: CASPER_EXFAT.as_bytes().to_vec(),
    });
    Ok(files)
}

fn boot_layout(
    dev_block: u32,
    volume: &fat32::Volume<'_>,
    files: &[BootFile],
) -> Result<fat32_tree::Layout, FormatError> {
    let new: Vec<NewFile> = files
        .iter()
        .map(|file| NewFile {
            path: file.path.split('/').map(str::to_owned).collect(),
            size: file.data.len() as u64,
        })
        .collect();
    fat32_tree::layout(volume, dev_block, &new, &[])
}

/// Data partition first, so systems that mount only the first partition of a removable drive show the
/// images; boot partition at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VaultPlan {
    pub table: TableKind,
    pub vault: PartitionPlan,
    pub efi: PartitionPlan,
}

/// FAT32 needs 65,525 clusters: 64 MiB with 512-byte sectors, about 257 MiB with 4 KiB ones.
fn efi_sectors(block: u32) -> Result<u64, FormatError> {
    let block = u64::from(block);
    let mut bytes = EFI_MIN_BYTES;
    loop {
        match fat32::geometry(bytes / block, u32::try_from(block).unwrap_or(0)) {
            Ok(_) => return Ok(bytes / block),
            Err(FormatError::TooSmall(_)) if bytes < EFI_MAX_BYTES => bytes += ALIGNMENT,
            Err(err) => return Err(err),
        }
    }
}

/// Where both partitions go, without writing: also used by `--dry-run`.
pub fn plan(size: u64, block: u32, table: TableKind) -> Result<VaultPlan, FormatError> {
    let too_small =
        || FormatError::TooSmall(format!("drive of {size} bytes too small for a vault"));
    let end = usable_end(size, block, table)?.ok_or_else(too_small)?;
    let align = ALIGNMENT.div_ceil(u64::from(block));
    let efi_first = (end + 1)
        .checked_sub(efi_sectors(block)?)
        .ok_or_else(too_small)?
        / align
        * align;
    if efi_first <= align || (efi_first - align) * u64::from(block) < VAULT_MIN_BYTES {
        return Err(too_small());
    }
    let plan = VaultPlan {
        table,
        vault: PartitionPlan {
            first_lba: align,
            last_lba: efi_first - 1,
        },
        efi: PartitionPlan {
            first_lba: efi_first,
            last_lba: end,
        },
    };
    exfat::geometry(plan.vault.sectors(), block)?;
    fat32::geometry(plan.efi.sectors(), block)?;
    Ok(plan)
}

/// GUID and volume id of the boot partition, derived from the drive ids.
fn efi_ids(ids: &Ids) -> ([u8; 16], u32) {
    let digest = Sha256::digest([&ids.partition_guid[..], b"efi"].concat());
    (
        guid(&digest[..16]),
        u32::from_le_bytes([digest[16], digest[17], digest[18], digest[19]]),
    )
}

fn exfat_volume(plan: &VaultPlan, ids: &Ids) -> exfat::Volume<'static> {
    exfat::Volume {
        first_lba: plan.vault.first_lba,
        sectors: plan.vault.sectors(),
        label: Some(VAULT_LABEL),
        volume_id: ids.volume_id,
    }
}

fn fat32_volume(plan: &VaultPlan, ids: &Ids) -> fat32::Volume<'static> {
    fat32::Volume {
        first_lba: plan.efi.first_lba,
        sectors: plan.efi.sectors(),
        label: Some(EFI_LABEL),
        volume_id: efi_ids(ids).1,
        bios_boot: false,
    }
}

/// Wipes old tables and partition starts, then writes the table. The plan must come from [`plan`] for this
/// drive: it is computed again and compared. Flushing is the caller's job.
pub fn write_table(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
) -> Result<(), FormatError> {
    if self::plan(dev.size(), dev.block_size(), plan.table)? != *plan {
        return Err(FormatError::Mismatch(
            "vault plan made for another drive".to_owned(),
        ));
    }
    let slots = [
        Slot {
            part: plan.vault,
            gpt_type: gpt_type(Filesystem::Exfat),
            mbr_type: mbr_type(Filesystem::Exfat),
            guid: ids.partition_guid,
            name: Some(VAULT_LABEL),
        },
        Slot {
            part: plan.efi,
            gpt_type: GUID_ESP,
            mbr_type: MBR_ESP,
            guid: efi_ids(ids).0,
            name: Some(EFI_NAME),
        },
    ];
    table::write_slots(dev, plan.table, &slots, ids, false)
}

/// Empty exFAT volume and FAT32 boot volume carrying `boot`, after [`write_table`].
pub fn write_volumes(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    exfat::write(dev, &exfat_volume(plan, ids))?;
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, boot)?;
    layout.write_structures(dev)?;
    for (index, file) in boot.iter().enumerate() {
        layout.write_file(dev, index, &mut file.data.as_slice(), &mut |_| true)?;
    }
    layout.finish(dev, &volume)
}

/// Table, both volumes and every boot file read back as written.
pub fn verify(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    if detect(dev)? != Some(*plan) {
        return Err(FormatError::Mismatch(
            "vault table read back differently".to_owned(),
        ));
    }
    exfat::verify(dev, &exfat_volume(plan, ids))?;
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, boot)?;
    layout.verify_structures(dev, &volume)?;
    for (index, file) in boot.iter().enumerate() {
        let expected = crate::write::hex(&Sha256::digest(&file.data));
        layout.verify_file(dev, index, &expected, &mut |_| true)?;
    }
    Ok(())
}

/// `Some` when the drive carries exactly the two vault partitions, the first an exFAT volume labelled
/// [`VAULT_LABEL`], the second typed as an EFI system partition.
pub fn detect(dev: &dyn BlockDevice) -> Result<Option<VaultPlan>, FormatError> {
    let layout = inspect(dev)?;
    if layout.logical_block_size != dev.block_size()
        || layout
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    {
        return Ok(None);
    }
    let found = match (&layout.gpt, &layout.mbr) {
        (Some(gpt), Some(mbr)) if mbr.protective => {
            let [vault, efi] = gpt.entries.as_slice() else {
                return Ok(None);
            };
            (efi.type_guid == ESP_GUID_TEXT).then_some((
                TableKind::Gpt,
                vault.first_lba,
                vault.last_lba,
                efi.first_lba,
                efi.last_lba,
            ))
        }
        (None, Some(mbr)) if !mbr.protective => {
            let [vault, efi] = mbr.entries.as_slice() else {
                return Ok(None);
            };
            let last = |entry: &MbrEntry| {
                (u64::from(entry.start_lba) + u64::from(entry.sectors)).checked_sub(1)
            };
            match (efi.kind == MBR_ESP, last(vault), last(efi)) {
                (true, Some(vault_last), Some(efi_last)) => Some((
                    TableKind::Mbr,
                    u64::from(vault.start_lba),
                    vault_last,
                    u64::from(efi.start_lba),
                    efi_last,
                )),
                _ => None,
            }
        }
        _ => None,
    };
    let Some((table, vault_first, vault_last, efi_first, efi_last)) = found else {
        return Ok(None);
    };
    if vault_last < vault_first || efi_last < efi_first {
        return Ok(None);
    }
    let plan = VaultPlan {
        table,
        vault: PartitionPlan {
            first_lba: vault_first,
            last_lba: vault_last,
        },
        efi: PartitionPlan {
            first_lba: efi_first,
            last_lba: efi_last,
        },
    };
    match Reader::open(dev, vault_first, plan.vault.sectors()) {
        Ok(reader) if reader.label()?.as_deref() == Some(VAULT_LABEL) => Ok(Some(plan)),
        Ok(_) | Err(FormatError::Invalid(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Files whose name ends in `.iso`, whatever the case, anywhere on the data partition.
pub fn images(dev: &dyn BlockDevice, plan: &VaultPlan) -> Result<Vec<FileEntry>, FormatError> {
    let files = Reader::open(dev, plan.vault.first_lba, plan.vault.sectors())?.files()?;
    Ok(files
        .into_iter()
        .filter(|file| {
            file.path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".iso"))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;
    use crate::format::NewFile;
    use crate::format::exfat_tree;

    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;

    fn ids() -> Ids {
        Ids::from_seed(b"vault")
    }

    fn boot() -> Vec<BootFile> {
        let files = boot_files(&[("x86_64", b"MZ grub x86_64".repeat(1000))]);
        assert!(files.is_ok(), "{files:?}");
        files.unwrap_or_default()
    }

    fn init(dev: &mut MemoryDevice, table: TableKind) -> Result<VaultPlan, FormatError> {
        let plan = plan(dev.size(), dev.block_size(), table)?;
        write_table(dev, &plan, &ids())?;
        write_volumes(dev, &plan, &ids(), &boot())?;
        Ok(plan)
    }

    #[test]
    fn familles_reconnues_comme_par_le_menu() -> std::io::Result<()> {
        use crate::iso9660::{Iso, testing::image};
        use std::io::Cursor;
        let family = |label: &str, files: &[(&str, u32)]| {
            iso_family(&mut Iso::open(Cursor::new(image(label, files)))?)
        };
        for (expected, files) in IsoFamily::RULES {
            // Every file the rule needs is one the menu tests.
            for file in files {
                assert!(MENU.contains(&format!("(probe)/{file}")), "{file}");
            }
            let present: Vec<(&str, u32)> = files.iter().map(|f| (*f, 1)).collect();
            assert_eq!(family("Fedora-WS-Live-44", &present)?, Some(expected));
            assert_eq!(family("X", &present[..1])?, None, "{expected:?}");
        }
        let kiwi = [
            ("boot/x86_64/loader/linux", 1),
            ("boot/x86_64/loader/initrd", 1),
        ];
        assert_eq!(family("openSUSE-Live", &kiwi)?, None);
        // Debian first, as in the menu, when an image carries both layouts.
        let both = [
            ("live/vmlinuz", 1),
            ("live/initrd.img", 1),
            ("casper/vmlinuz", 1),
            ("casper/initrd", 1),
        ];
        assert_eq!(family("X", &both)?, Some(IsoFamily::DebianLive));
        Ok(())
    }

    #[test]
    fn plan_mbr_exfat_d_abord_puis_esp_de_64_mio_en_fin() -> Result<(), FormatError> {
        let plan = plan(GIB, 512, TableKind::Mbr)?;
        assert_eq!(plan.vault.first_lba, 2048);
        assert_eq!(plan.efi.last_lba, GIB / 512 - 1);
        assert_eq!(plan.efi.sectors() * 512, 64 * MIB);
        assert_eq!(plan.vault.last_lba + 1, plan.efi.first_lba);
        assert_eq!(plan.efi.first_lba % 2048, 0);
        Ok(())
    }

    #[test]
    fn plan_gpt_laisse_la_place_de_la_table_de_secours() -> Result<(), FormatError> {
        let plan = plan(GIB, 512, TableKind::Gpt)?;
        assert_eq!(plan.efi.last_lba, GIB / 512 - 34);
        assert!(plan.efi.sectors() * 512 >= 64 * MIB);
        Ok(())
    }

    #[test]
    fn esp_agrandie_sur_secteurs_de_4_kio() -> Result<(), FormatError> {
        let plan = plan(2 * GIB, 4096, TableKind::Mbr)?;
        let bytes = plan.efi.sectors() * 4096;
        assert!(bytes > 256 * MIB && bytes < 300 * MIB, "{bytes}");
        assert!(fat32::geometry(plan.efi.sectors(), 4096).is_ok());
        Ok(())
    }

    #[test]
    fn refuse_trop_petit_et_mbr_trop_grand() {
        assert!(matches!(
            plan(100 * MIB, 512, TableKind::Mbr),
            Err(FormatError::TooSmall(_))
        ));
        assert!(matches!(
            plan(4 << 40, 512, TableKind::Mbr),
            Err(FormatError::TooLarge(_))
        ));
        assert!(plan(4 << 40, 512, TableKind::Gpt).is_ok());
    }

    #[test]
    fn ecrit_relu_et_reconnu_en_mbr_et_gpt() -> Result<(), Box<dyn std::error::Error>> {
        for table in TableKind::ALL {
            let mut dev = MemoryDevice::new(256 * MIB, 512)?;
            let plan = init(&mut dev, table)?;
            verify(&dev, &plan, &ids(), &boot())?;
            assert_eq!(detect(&dev)?, Some(plan));
            assert!(images(&dev, &plan)?.is_empty());
            let layout = inspect(&dev)?;
            assert!(
                layout
                    .findings
                    .iter()
                    .all(|finding| finding.severity == Severity::Info),
                "{:?}",
                layout.findings
            );
        }
        Ok(())
    }

    #[test]
    fn types_et_noms_des_partitions() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        init(&mut dev, TableKind::Gpt)?;
        let gpt = inspect(&dev)?.gpt.ok_or("pas de GPT")?;
        assert_eq!(gpt.entries[0].name, "BonkVault");
        assert_eq!(gpt.entries[1].name, "BonkEFI");
        assert_eq!(gpt.entries[1].type_guid, ESP_GUID_TEXT);
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        init(&mut dev, TableKind::Mbr)?;
        let mbr = inspect(&dev)?.mbr.ok_or("pas de MBR")?;
        let kinds: Vec<u8> = mbr.entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [0x07, 0xEF]);
        assert!(mbr.entries.iter().all(|e| !e.bootable));
        Ok(())
    }

    #[test]
    fn une_cle_formatee_n_est_pas_un_coffre() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let part = table::write_table(
            &mut dev,
            TableKind::Mbr,
            Filesystem::Exfat,
            Some(VAULT_LABEL),
            &ids(),
            false,
        )?;
        exfat::write(
            &mut dev,
            &exfat::Volume {
                first_lba: part.first_lba,
                sectors: part.sectors(),
                label: Some(VAULT_LABEL),
                volume_id: 1,
            },
        )?;
        assert_eq!(detect(&dev)?, None);
        assert_eq!(detect(&MemoryDevice::new(8 * MIB, 512)?)?, None);
        Ok(())
    }

    #[test]
    fn liste_les_iso_sans_les_autres_fichiers() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let plan = init(&mut dev, TableKind::Mbr)?;
        let file = |path: &str, size| NewFile {
            path: path.split('/').map(str::to_owned).collect(),
            size,
        };
        let volume = exfat_volume(&plan, &ids());
        let layout = exfat_tree::layout(
            &volume,
            512,
            &[
                file("debian-13.iso", 4 * MIB),
                file("Linux/Fedora.ISO", MIB),
                file("Linux/lisez-moi.txt", 10),
            ],
            &[],
        )?;
        layout.write_structures(&mut dev)?;
        layout.finish(&mut dev, &volume)?;
        assert_eq!(detect(&dev)?, Some(plan));
        let found: Vec<String> = images(&dev, &plan)?.into_iter().map(|f| f.path).collect();
        assert_eq!(found, ["Linux/Fedora.ISO", "debian-13.iso"]);
        Ok(())
    }

    #[test]
    fn les_fichiers_de_demarrage_portent_les_noms_uefi_et_le_menu() -> Result<(), FormatError> {
        let files = boot_files(&[
            ("x86_64", vec![1]),
            ("aarch64", vec![2]),
            ("riscv64", vec![3]),
        ])?;
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "EFI/BOOT/BOOTX64.EFI",
                "EFI/BOOT/BOOTAA64.EFI",
                "EFI/BOOT/BOOTRISCV64.EFI",
                "bonk/grub.cfg",
                "bonk/casper/20iso_scan"
            ]
        );
        assert!(files[3].data.starts_with(b"set timeout="));
        assert!(matches!(
            boot_files(&[("mips", vec![])]),
            Err(FormatError::Unsupported(_))
        ));
        Ok(())
    }

    /// The read-back compares every boot file: a changed byte of GRUB is caught.
    #[test]
    fn un_octet_de_grub_change_fait_echouer_la_relecture() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let plan = init(&mut dev, TableKind::Mbr)?;
        let mut other = boot();
        other[0].data[10] ^= 1;
        assert!(verify(&dev, &plan, &ids(), &other).is_err());
        verify(&dev, &plan, &ids(), &boot())?;
        Ok(())
    }

    #[test]
    fn un_plan_pour_un_autre_disque_n_ecrit_rien() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let other = plan(GIB, 512, TableKind::Mbr)?;
        assert!(matches!(
            write_table(&mut dev, &other, &ids()),
            Err(FormatError::Mismatch(_))
        ));
        assert!(dev.as_bytes().iter().all(|b| *b == 0));
        Ok(())
    }
}
