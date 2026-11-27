use schemars::JsonSchema;
use serde::Serialize;

use super::VaultFamily;
use crate::iso9660::Iso;
use crate::read_at::{Cursor, ReadAt};

/// The processor an ISO starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IsoArch {
    X86_64,
    Aarch64,
}

/// Tested in the menu's order: a Kiwi kernel folder names its processor; Debian and Ubuntu share one layout
/// across processors, so their own UEFI loader tells. GRUB compares Rock Ridge names with their case, Bonk
/// without: these are spelled as the images spell them.
pub(super) const ARCH_RULES: [(&str, IsoArch); 4] = [
    ("boot/aarch64/loader/linux", IsoArch::Aarch64),
    ("boot/x86_64/loader/linux", IsoArch::X86_64),
    ("EFI/boot/bootaa64.efi", IsoArch::Aarch64),
    ("EFI/BOOT/BOOTAA64.EFI", IsoArch::Aarch64),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoKind {
    pub family: VaultFamily,
    pub arch: IsoArch,
}

/// The vault family that starts an image, as `bonk info` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VaultMatch {
    /// Identifier of the family, `debian_live`.
    pub family: String,
    pub name: String,
    pub arch: IsoArch,
}

/// `None` for an image that is not an ISO 9660 one, or that no family starts.
///
/// # Errors
/// Unreadable image.
pub fn vault_match<S: ReadAt + ?Sized>(
    source: &S,
    len: u64,
    families: &[VaultFamily],
) -> std::io::Result<Option<VaultMatch>> {
    let mut iso = match Iso::open(Cursor::new(source, len)) {
        Ok(iso) => iso,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    Ok(iso_kind(&mut iso, families)?.map(|kind| VaultMatch {
        family: kind.family.id,
        name: kind.family.name,
        arch: kind.arch,
    }))
}

/// What the vault menu would start, `None` for an ISO it shows as not supported.
///
/// # Errors
/// Not an ISO 9660 image, or unreadable.
pub fn iso_kind<R: std::io::Read + std::io::Seek>(
    iso: &mut Iso<R>,
    families: &[VaultFamily],
) -> std::io::Result<Option<IsoKind>> {
    // Families sharing a layout (Kiwi) differ by their label: one that does not fit lets the next one try.
    for family in families {
        let label_fits = family
            .label_prefix
            .as_deref()
            .is_none_or(|prefix| iso.label().starts_with(prefix));
        if !label_fits || !has_any_set(iso, &family.detect)? {
            continue;
        }
        return Ok(Some(IsoKind {
            family: family.clone(),
            arch: iso_arch(iso)?,
        }));
    }
    Ok(None)
}

fn iso_arch<R: std::io::Read + std::io::Seek>(iso: &mut Iso<R>) -> std::io::Result<IsoArch> {
    for (file, arch) in ARCH_RULES {
        if iso.file_size(file)?.is_some() {
            return Ok(arch);
        }
    }
    Ok(IsoArch::X86_64)
}

fn has_any_set<R: std::io::Read + std::io::Seek>(
    iso: &mut Iso<R>,
    sets: &[Vec<String>],
) -> std::io::Result<bool> {
    for set in sets {
        let mut all = true;
        for file in set {
            all &= iso.file_size(file)?.is_some();
        }
        if all {
            return Ok(true);
        }
    }
    Ok(false)
}
