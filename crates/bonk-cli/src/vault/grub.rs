use std::fs;
use std::path::Path;

use bonk_catalog::manifest::{self, GrubFile};
use bonk_core::vault::{self as core_vault, GrubInfo, VaultBoot};
use bonk_ops::job::JobError;

use super::GrubSource;
use crate::catalog;

const BIOS_BOOT: &str = "bios/boot.img";
const BIOS_CORE: &str = "bios/core.img";

fn file_name(file: GrubFile<'_>) -> &'static str {
    match file {
        GrubFile::Efi(arch) => core_vault::efi_file_name(arch).unwrap_or_default(),
        GrubFile::BiosBoot => BIOS_BOOT,
        GrubFile::BiosCore => BIOS_CORE,
    }
}

pub(crate) fn grub(
    args: &GrubSource,
    access: &catalog::Access,
) -> Result<(VaultBoot, GrubInfo), JobError> {
    let (loaded, dir, base) = catalog::load_from(
        access,
        args.catalog_url.as_deref(),
        args.cache_dir.as_deref(),
    )?;
    let entry = loaded
        .catalog
        .grub
        .ok_or_else(|| catalog::refused("the catalog does not offer GRUB yet: try again later"))?;
    let fetch = |name: &str, asset: &manifest::Asset| match &args.grub_dir {
        Some(folder) => local_grub(&folder.join(name), asset, &entry.version),
        None => {
            let path = manifest::asset_path(&base, &dir, "grub", asset, &*access.fetch_file)
                .map_err(|e| catalog::refused(format!("GRUB {} {name}: {e}", entry.version)))?;
            fs::read(&path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))
        }
    };
    let read = |file: GrubFile<'_>, asset: &manifest::Asset| {
        fetch(file_name(file), asset).map_err(|e| e.message)
    };
    // A folder copied before the catalog offered GRUB for BIOS gives a vault that starts by UEFI only.
    let copied_without_bios = args
        .grub_dir
        .as_ref()
        .is_some_and(|folder| !folder.join(BIOS_CORE).exists());
    let bios = entry.bios.as_ref().filter(|_| !copied_without_bios);
    let families = match &args.families {
        Some(file) => catalog::families_file(file)?,
        None => core_vault::families_from(&loaded.catalog.vault_families),
    };
    entry
        .vault_boot(bios, &families, &read)
        .map_err(catalog::refused)
}

pub(super) fn local_grub(
    path: &Path,
    asset: &manifest::Asset,
    version: &str,
) -> Result<Vec<u8>, JobError> {
    let data = fs::read(path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))?;
    if !manifest::asset_matches(asset, &data) {
        return Err(catalog::refused(format!(
            "{} is not the GRUB {version} of the catalog (hash differs)",
            path.display()
        )));
    }
    Ok(data)
}
