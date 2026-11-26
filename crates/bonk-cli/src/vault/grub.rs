use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use bonk_catalog::manifest;
use bonk_core::vault::{self as core_vault, BiosGrub, VaultBoot, VaultFamily};
use bonk_ops::job::{GrubInfo, JobError};

use super::GrubSource;
use crate::catalog;

const BIOS_BOOT: &str = "bios/boot.img";
const BIOS_CORE: &str = "bios/core.img";

type Read<'a> = dyn Fn(&str, &manifest::Asset) -> Result<Vec<u8>, JobError> + 'a;

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
    let read = |name: &str, asset: &manifest::Asset| match &args.grub_dir {
        Some(folder) => local_grub(&folder.join(name), asset, &entry.version),
        None => {
            let path = manifest::asset_path(&base, &dir, "grub", asset, &*access.fetch_file)
                .map_err(|e| catalog::refused(format!("GRUB {} {name}: {e}", entry.version)))?;
            fs::read(&path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))
        }
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
    vault_boot(&entry, bios, &families, &read)
}

fn vault_boot(
    entry: &manifest::Grub,
    bios: Option<&manifest::GrubBios>,
    families: &[VaultFamily],
    read: &Read<'_>,
) -> Result<(VaultBoot, GrubInfo), JobError> {
    let mut images = Vec::new();
    let mut efi = BTreeMap::new();
    // A processor this Bonk has no removable-media name for is left to a later version.
    for (arch, asset) in &entry.efi {
        let Some(name) = core_vault::efi_file_name(arch) else {
            continue;
        };
        efi.insert(arch.clone(), asset.sha256.clone());
        images.push((arch.as_str(), read(name, asset)?));
    }
    if images.is_empty() {
        return Err(catalog::refused(
            "the catalog offers no GRUB image this version of Bonk can write",
        ));
    }
    let files =
        core_vault::boot_files(&images, families).map_err(|e| catalog::refused(e.to_string()))?;
    let bios_grub = match bios {
        Some(offered) => Some(BiosGrub {
            boot: read(BIOS_BOOT, &offered.boot)?,
            core: read(BIOS_CORE, &offered.core)?,
        }),
        None => None,
    };
    let info = GrubInfo {
        version: entry.version.clone(),
        build: entry.build.clone(),
        efi,
        bios: bios.map(manifest::GrubBios::hashes).unwrap_or_default(),
    };
    let boot = VaultBoot {
        files,
        bios: bios_grub,
    };
    Ok((boot, info))
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
