mod boot;
mod images;
mod iso;
mod plan;
#[cfg(test)]
mod tests;

pub use boot::*;
pub use images::*;
pub use iso::*;
pub use plan::*;
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
pub const MENU: &str = include_str!("menu.cfg");
/// The path the GRUB image searches for.
pub const MENU_PATH: &str = "bonk/grub.cfg";
/// Ubuntu's iso-scan step, put into its initrd by the menu: it maps the ISO from the partition, which casper
/// cannot read. Kept apart, as it follows casper's own scripts.
pub const CASPER_EXFAT: &str = include_str!("casper_exfat.sh");
/// Next to the menu, which keeps GRUB's `config_directory` to name it.
pub const CASPER_EXFAT_PATH: &str = "bonk/casper/20iso_scan";
