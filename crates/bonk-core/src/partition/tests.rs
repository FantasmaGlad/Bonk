use super::testing::{ESP, LINUX, gpt_image, put_u32, put_u64};
use super::*;
use crate::block::MemoryDevice;

fn inspect_bytes(bytes: Vec<u8>) -> Result<Layout, BlockError> {
    inspect(&MemoryDevice::from_bytes(bytes, 512)?)
}

fn codes(layout: &Layout) -> Vec<&'static str> {
    layout.findings.iter().map(|f| f.code).collect()
}

#[test]
fn crc32_valeurs_de_reference() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b"a"), 0xE8B7_BE43);
}

#[test]
fn gpt_valide_sans_constat_grave() -> Result<(), Box<dyn std::error::Error>> {
    let layout = inspect_bytes(gpt_image(
        4096,
        &[(2048, 2559, ESP, "EFI"), (2560, 4000, LINUX, "racine")],
    ))?;
    assert_eq!(layout.logical_block_size, 512);
    let gpt = layout.gpt.as_ref().ok_or("GPT attendu")?;
    let (primary, backup) = (
        gpt.primary.as_ref().ok_or("principal")?,
        gpt.backup.as_ref().ok_or("secours")?,
    );
    assert!(primary.header_crc_ok && primary.entries_crc_ok);
    assert!(backup.header_crc_ok && backup.entries_crc_ok);
    assert_eq!(primary.disk_guid, "D1D1D1D1-D1D1-D1D1-D1D1-D1D1D1D1D1D1");
    assert_eq!(gpt.entries.len(), 2);
    assert_eq!(
        gpt.entries[0].type_guid,
        "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"
    );
    assert_eq!(gpt.entries[0].name, "EFI");
    assert_eq!(gpt.entries[1].name, "racine");
    let mbr = layout.mbr.as_ref().ok_or("MBR attendu")?;
    assert!(mbr.signature_valid && mbr.protective);
    // 2048 secteurs de 512 octets = 1 Mio : aligné ; rien d'anormal.
    assert!(
        layout.findings.iter().all(|f| f.severity == Severity::Info),
        "{:?}",
        layout.findings
    );
    Ok(())
}

#[test]
fn en_tete_principal_corrompu() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    img[512 + 40] ^= 0xFF; // premier secteur utilisable modifié sans recalcul de la somme
    let layout = inspect_bytes(img)?;
    assert!(
        codes(&layout).contains(&"gpt_primary_header_crc"),
        "{:?}",
        codes(&layout)
    );
    // Le secours reste valide : la réparation sera possible.
    assert!(!codes(&layout).contains(&"gpt_backup_header_crc"));
    Ok(())
}

#[test]
fn table_principale_corrompue() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    img[2 * 512 + 40] ^= 0x01; // un octet de la table
    let layout = inspect_bytes(img)?;
    assert!(codes(&layout).contains(&"gpt_primary_entries_crc"));
    assert!(!codes(&layout).contains(&"gpt_backup_entries_crc"));
    // Les entrées rendues viennent de la table de secours, saine.
    assert_eq!(
        layout.gpt.as_ref().map(|g| g.entries[0].last_lba),
        Some(4000)
    );
    Ok(())
}

#[test]
fn secours_absent_quand_le_disque_est_plus_grand() -> Result<(), Box<dyn std::error::Error>> {
    let small = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    let mut big = small.clone();
    big.resize(8192 * 512, 0);
    let layout = inspect_bytes(big)?;
    let found = codes(&layout);
    assert!(
        found.contains(&"gpt_backup_not_at_end") || found.contains(&"gpt_backup_missing"),
        "{found:?}"
    );
    Ok(())
}

#[test]
fn secours_efface_est_signale() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    let last = 4095 * 512;
    img[last..last + 512].fill(0);
    let layout = inspect_bytes(img)?;
    assert!(codes(&layout).contains(&"gpt_backup_missing"));
    Ok(())
}

#[test]
fn partitions_qui_se_chevauchent_ou_sortent_de_la_zone() -> Result<(), Box<dyn std::error::Error>> {
    let layout = inspect_bytes(gpt_image(
        4096,
        &[
            (2048, 3000, LINUX, "a"),
            (2900, 3500, LINUX, "b"),
            (10, 20, LINUX, "c"),
        ],
    ))?;
    let found = codes(&layout);
    assert!(found.contains(&"gpt_entries_overlap"), "{found:?}");
    assert!(found.contains(&"gpt_entry_out_of_range"), "{found:?}");
    Ok(())
}

#[test]
fn partition_non_alignee_est_une_information() -> Result<(), Box<dyn std::error::Error>> {
    let layout = inspect_bytes(gpt_image(4096, &[(2049, 4000, LINUX, "x")]))?;
    let f = layout
        .findings
        .iter()
        .find(|f| f.code == "gpt_entry_unaligned");
    assert!(f.is_some_and(|f| f.severity == Severity::Info));
    Ok(())
}

#[test]
fn gpt_sans_mbr_de_protection() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    img[446 + 4] = 0;
    let layout = inspect_bytes(img)?;
    assert!(codes(&layout).contains(&"gpt_protective_mbr_missing"));
    Ok(())
}

#[test]
fn mbr_classique() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = vec![0u8; 4096 * 512];
    img[510] = 0x55;
    img[511] = 0xAA;
    for (i, (start, len, kind)) in [(2048u32, 1000u32, 0x0Cu8), (3048, 500, 0x83)]
        .into_iter()
        .enumerate()
    {
        let at = 446 + i * 16;
        img[at] = if i == 0 { 0x80 } else { 0 };
        img[at + 4] = kind;
        put_u32(&mut img, at + 8, start);
        put_u32(&mut img, at + 12, len);
    }
    let layout = inspect_bytes(img)?;
    assert!(layout.gpt.is_none());
    let mbr = layout.mbr.as_ref().ok_or("MBR attendu")?;
    assert_eq!(mbr.entries.len(), 2);
    assert!(mbr.entries[0].bootable && !mbr.entries[1].bootable);
    assert_eq!(
        (mbr.entries[1].kind, mbr.entries[1].start_lba),
        (0x83, 3048)
    );
    assert!(layout.findings.is_empty(), "{:?}", layout.findings);
    Ok(())
}

#[test]
fn mbr_avec_entree_hors_du_disque_ou_chevauchante() -> Result<(), Box<dyn std::error::Error>> {
    let mut img = vec![0u8; 4096 * 512];
    img[510] = 0x55;
    img[511] = 0xAA;
    for (i, (start, len)) in [(100u32, 500u32), (300, 200), (4000, 500)]
        .into_iter()
        .enumerate()
    {
        let at = 446 + i * 16;
        img[at + 4] = 0x83;
        put_u32(&mut img, at + 8, start);
        put_u32(&mut img, at + 12, len);
    }
    let layout = inspect_bytes(img)?;
    let found = codes(&layout);
    assert!(
        found.contains(&"mbr_entries_overlap") && found.contains(&"mbr_entry_out_of_range"),
        "{found:?}"
    );
    Ok(())
}

#[test]
fn disque_vierge_et_image_minuscule() -> Result<(), Box<dyn std::error::Error>> {
    let blank = inspect_bytes(vec![0u8; 4096 * 512])?;
    assert!(blank.mbr.is_none() && blank.gpt.is_none());
    assert_eq!(codes(&blank), ["no_partition_table"]);
    let tiny = inspect(&MemoryDevice::new(512, 512)?)?;
    assert_eq!(codes(&tiny), ["too_small"]);
    Ok(())
}

#[test]
fn secteur_logique_de_4096_octets_detecte() -> Result<(), Box<dyn std::error::Error>> {
    // Un en-tête GPT au secteur 1 d'un disque à secteurs de 4096 octets se trouve à l'octet 4096.
    let sectors = 64usize;
    let bs = 4096usize;
    let mut img = vec![0u8; sectors * bs];
    let h = &mut img[bs..bs + 512];
    h[..8].copy_from_slice(GPT_SIGNATURE);
    put_u32(h, 12, 92);
    put_u64(h, 24, 1);
    put_u64(h, 32, (sectors - 1) as u64);
    put_u64(h, 72, 2);
    put_u32(h, 80, 128);
    put_u32(h, 84, 128);
    let layout = inspect_bytes(img)?;
    assert_eq!(layout.logical_block_size, 4096);
    assert!(layout.gpt.as_ref().is_some_and(|g| g.primary.is_some()));
    Ok(())
}

#[test]
fn table_aberrante_ne_fait_ni_lire_hors_bornes_ni_paniquer()
-> Result<(), Box<dyn std::error::Error>> {
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    // Nombre d'entrées et taille démesurés dans l'en-tête principal.
    put_u32(&mut img, 512 + 80, u32::MAX);
    put_u32(&mut img, 512 + 84, u32::MAX);
    let layout = inspect_bytes(img)?;
    assert!(codes(&layout).contains(&"gpt_primary_entries_crc"));
    // Début de table hors du disque.
    let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
    put_u64(&mut img, 512 + 72, u64::MAX / 2);
    assert!(inspect_bytes(img).is_ok());
    Ok(())
}

/// Le début de partition vient de l'image : le multiplier par la taille de bloc peut déborder.
#[test]
fn un_debut_de_partition_demesure_ne_fait_pas_paniquer() -> Result<(), Box<dyn std::error::Error>> {
    let huge = 1u64 << 62;
    let img = gpt_image(4096, &[(huge, huge + 1, LINUX, "x")]);
    assert!(codes(&inspect_bytes(img)?).contains(&"gpt_entry_out_of_range"));
    Ok(())
}
