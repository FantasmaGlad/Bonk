use schemars::JsonSchema;
use serde::Serialize;

use super::VaultFamily;

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

/// What the vault menu would start, `None` for an ISO it shows as not supported.
///
/// # Errors
/// Not an ISO 9660 image, or unreadable.
pub fn iso_kind<R: std::io::Read + std::io::Seek>(
    iso: &mut crate::iso9660::Iso<R>,
    families: &[VaultFamily],
) -> std::io::Result<Option<IsoKind>> {
    for family in families {
        if !has_any_set(iso, &family.detect)? {
            continue;
        }
        if family
            .label_prefix
            .as_deref()
            .is_some_and(|prefix| !iso.label().starts_with(prefix))
        {
            return Ok(None);
        }
        return Ok(Some(IsoKind {
            family: family.clone(),
            arch: iso_arch(iso)?,
        }));
    }
    Ok(None)
}

fn iso_arch<R: std::io::Read + std::io::Seek>(
    iso: &mut crate::iso9660::Iso<R>,
) -> std::io::Result<IsoArch> {
    for (file, arch) in ARCH_RULES {
        if iso.file_size(file)?.is_some() {
            return Ok(arch);
        }
    }
    Ok(IsoArch::X86_64)
}

fn has_any_set<R: std::io::Read + std::io::Seek>(
    iso: &mut crate::iso9660::Iso<R>,
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
