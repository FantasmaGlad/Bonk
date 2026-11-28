use bonk_core::vault::{self as core_vault, BiosGrub, GrubInfo, VaultBoot, VaultFamily};

use super::{Asset, Grub, GrubBios};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrubFile<'a> {
    Efi(&'a str),
    BiosBoot,
    BiosCore,
}

pub type GrubRead<'r> = dyn Fn(GrubFile<'_>, &Asset) -> Result<Vec<u8>, String> + 'r;

impl Grub {
    pub fn writable_efi(&self) -> impl Iterator<Item = (&String, &Asset)> {
        self.efi
            .iter()
            .filter(|(arch, _)| core_vault::efi_file_name(arch).is_some())
    }

    /// # Errors
    /// A file `read` refuses, no image this Bonk can write, or files the vault cannot hold.
    pub fn vault_boot(
        &self,
        bios: Option<&GrubBios>,
        families: &[VaultFamily],
        read: &GrubRead<'_>,
    ) -> Result<(VaultBoot, GrubInfo), String> {
        let mut images = Vec::new();
        for (arch, asset) in self.writable_efi() {
            images.push((arch.as_str(), read(GrubFile::Efi(arch), asset)?));
        }
        if images.is_empty() {
            return Err(
                "the catalog offers no GRUB image this version of Bonk can write".to_owned(),
            );
        }
        let files = core_vault::boot_files(&images, families).map_err(|e| e.to_string())?;
        let bios_grub = match bios {
            Some(offered) => Some(BiosGrub {
                boot: read(GrubFile::BiosBoot, &offered.boot)?,
                core: read(GrubFile::BiosCore, &offered.core)?,
            }),
            None => None,
        };
        let info = GrubInfo {
            version: self.version.clone(),
            build: self.build.clone(),
            efi: self
                .writable_efi()
                .map(|(arch, asset)| (arch.clone(), asset.sha256.clone()))
                .collect(),
            bios: bios.map(GrubBios::hashes).unwrap_or_default(),
        };
        Ok((
            VaultBoot {
                files,
                bios: bios_grub,
            },
            info,
        ))
    }
}

#[cfg(test)]
mod tests;
