use super::*;

/// GRUB named by the signed catalog, one image per processor Bonk can write: from its address through the
/// cache, or from `--grub-dir`, always checked against the signed hash.
pub(crate) fn grub(
    args: &GrubSource,
    access: &catalog::Access,
) -> Result<(Vec<BootFile>, GrubInfo), JobError> {
    let (loaded, dir, base) = catalog::load_from(
        access,
        args.catalog_url.as_deref(),
        args.cache_dir.as_deref(),
    )?;
    let entry = loaded
        .catalog
        .grub
        .ok_or_else(|| catalog::refused("the catalog does not offer GRUB yet: try again later"))?;
    let mut images = Vec::new();
    let mut efi = BTreeMap::new();
    // A processor this Bonk has no removable-media name for is left to a later version.
    for (arch, asset) in &entry.efi {
        let Some(name) = core_vault::efi_file_name(arch) else {
            continue;
        };
        let data = match &args.grub_dir {
            Some(folder) => local_grub(&folder.join(name), asset, &entry.version)?,
            None => {
                let path = manifest::asset_path(&base, &dir, "grub", asset, &*access.fetch_file)
                    .map_err(|e| catalog::refused(format!("GRUB {} {arch}: {e}", entry.version)))?;
                fs::read(&path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))?
            }
        };
        efi.insert(arch.clone(), asset.sha256.clone());
        images.push((arch.as_str(), data));
    }
    if images.is_empty() {
        return Err(catalog::refused(
            "the catalog offers no GRUB image this version of Bonk can write",
        ));
    }
    let files = core_vault::boot_files(&images).map_err(|e| catalog::refused(e.to_string()))?;
    let info = GrubInfo {
        version: entry.version,
        build: entry.build,
        efi,
    };
    Ok((files, info))
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
