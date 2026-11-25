mod boot;
mod family;
mod images;
mod iso;
mod menu;
mod plan;
#[cfg(test)]
mod tests;

pub use boot::*;
pub use family::*;
pub use images::*;
pub use iso::*;
pub use menu::menu;
pub use plan::*;
/// exFAT volume label and partition name of the data partition.
pub const VAULT_LABEL: &str = "BonkVault";
pub const EFI_NAME: &str = "BonkEFI";
pub const EFI_LABEL: &str = "BONKEFI";
const EFI_MIN_BYTES: u64 = 64 << 20;
const EFI_MAX_BYTES: u64 = 1 << 30;
const VAULT_MIN_BYTES: u64 = 64 << 20;
pub const MENU_PATH: &str = "bonk/grub.cfg";
pub const FAMILIES_PATH: &str = "bonk/families.json";
/// Ubuntu's iso-scan step, in place of casper's own.
pub const CASPER_SCRIPT: &str = include_str!("initrd/casper.sh");
/// A udev rule for the installers of the Red Hat family: their dracut hooks moved between versions, udev's
/// rules folder did not.
pub const VAULT_ISO_RULE: &str = include_str!("initrd/vault-iso.rules");
/// Scripts the menu puts into an initrd, by their path next to the menu: each maps an ISO from the partition,
/// which the system started cannot read. Kept apart, as each follows its initramfs's own scripts.
pub const INITRD_SCRIPTS: [(&str, &str); 2] = [
    ("bonk/casper/20iso_scan", CASPER_SCRIPT),
    ("bonk/udev/99-bonk-vault-iso.rules", VAULT_ISO_RULE),
];
