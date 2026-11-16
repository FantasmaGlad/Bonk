use schemars::JsonSchema;
use serde::Serialize;

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
    pub(super) const RULES: [(Self, &'static [&'static str]); 5] = [
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
        (
            Self::FedoraLive,
            &["boot/aarch64/loader/linux", "boot/aarch64/loader/initrd"],
        ),
    ];
}

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
pub(super) const ARCH_RULES: [(&str, IsoArch); 3] = [
    ("boot/aarch64/loader/linux", IsoArch::Aarch64),
    ("boot/x86_64/loader/linux", IsoArch::X86_64),
    ("EFI/boot/bootaa64.efi", IsoArch::Aarch64),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsoKind {
    pub family: IsoFamily,
    pub arch: IsoArch,
}

/// What the vault menu would start, `None` for an ISO it shows as not supported.
///
/// # Errors
/// Not an ISO 9660 image, or unreadable.
pub fn iso_kind<R: std::io::Read + std::io::Seek>(
    iso: &mut crate::iso9660::Iso<R>,
) -> std::io::Result<Option<IsoKind>> {
    for (family, files) in IsoFamily::RULES {
        let mut all = true;
        for file in files {
            all &= iso.file_size(file)?.is_some();
        }
        if all {
            // The menu checks the label too: other Kiwi images share this layout.
            if family == IsoFamily::FedoraLive && !iso.label().starts_with("Fedora-") {
                return Ok(None);
            }
            let mut arch = IsoArch::X86_64;
            for (file, found) in ARCH_RULES {
                if iso.file_size(file)?.is_some() {
                    arch = found;
                    break;
                }
            }
            return Ok(Some(IsoKind { family, arch }));
        }
    }
    Ok(None)
}
