use crate::block::MemoryDevice;
use crate::format::exfat::edit::*;
use crate::format::exfat::{self, Volume as NewVolume};
use crate::partition::read_range;

const MIB: u64 = 1 << 20;
const VOLUME: NewVolume<'static> = NewVolume {
    first_lba: 2048,
    sectors: 64 * MIB / 512,
    label: Some("BonkVault"),
    volume_id: 7,
};
const SPAN: Span = Span {
    first_lba: VOLUME.first_lba,
    sectors: VOLUME.sectors,
};

fn empty() -> Result<MemoryDevice, FormatError> {
    let mut dev = MemoryDevice::new(66 * MIB, 512).map_err(FormatError::from)?;
    exfat::write(&mut dev, &VOLUME)?;
    Ok(dev)
}

fn add(dev: &mut MemoryDevice, name: &str, data: &[u8]) -> Result<Added, FormatError> {
    let mut reader = data;
    add_file(
        dev,
        SPAN,
        name,
        data.len() as u64,
        false,
        &mut reader,
        &mut |_| true,
    )
}

fn listed(dev: &MemoryDevice) -> Result<Vec<(String, u64)>, FormatError> {
    Ok(Reader::open(dev, VOLUME.first_lba, VOLUME.sectors)?
        .files()?
        .into_iter()
        .map(|file| (file.path, file.size_bytes))
        .collect())
}

fn content(dev: &MemoryDevice, name: &str) -> Result<Vec<u8>, FormatError> {
    let volume = Volume::read(dev, VOLUME.first_lba, VOLUME.sectors)?;
    let raw = Reader::entries(&volume.root_bytes, &mut None)
        .into_iter()
        .find(|raw| raw.name == name)
        .ok_or_else(|| invalid("absent"))?;
    Ok(read_range(
        dev,
        volume.cluster_offset(raw.first_cluster),
        raw.length,
    )?)
}

#[test]
fn un_fichier_ajoute_est_relu_contigu_et_le_volume_reste_propre() -> Result<(), FormatError> {
    let mut dev = empty()?;
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7) as u8).collect();
    let added = add(&mut dev, "debian-live.iso", &data)?;
    assert_eq!(added.size_bytes, data.len() as u64);
    verify_file(&dev, SPAN, "debian-live.iso", &added.sha256, &mut |_| true)?;
    assert!(
        verify_file(&dev, SPAN, "debian-live.iso", &"0".repeat(64), &mut |_| {
            true
        })
        .is_err()
    );
    add(&mut dev, "vide.txt", &[])?;
    assert_eq!(
        listed(&dev)?,
        [
            ("debian-live.iso".to_owned(), data.len() as u64),
            ("vide.txt".to_owned(), 0)
        ]
    );
    assert_eq!(content(&dev, "debian-live.iso")?, data);
    let at = VOLUME.first_lba * 512 + added.volume_offset;
    assert_eq!(read_range(&dev, at, data.len() as u64)?, data);
    let start = VOLUME.first_lba * 512;
    assert_eq!(
        read_range(&dev, start + VOLUME_FLAGS, 1)?[0] & VOLUME_DIRTY,
        0
    );
    Ok(())
}

#[test]
fn un_nom_deja_present_ou_refuse_n_ecrit_rien() -> Result<(), FormatError> {
    let mut dev = empty()?;
    add(&mut dev, "Ubuntu.iso", b"abc")?;
    let before = dev.as_bytes().to_vec();
    assert!(matches!(
        add(&mut dev, "UBUNTU.ISO", b"x"),
        Err(FormatError::Name(_))
    ));
    assert!(matches!(
        add(&mut dev, "a/b.iso", b"x"),
        Err(FormatError::Name(_))
    ));
    assert_eq!(dev.as_bytes(), before.as_slice());
    Ok(())
}

#[test]
fn sans_place_contigue_rien_n_est_ecrit() -> Result<(), FormatError> {
    let mut dev = empty()?;
    let before = dev.as_bytes().to_vec();
    let mut reader = std::io::repeat(0);
    let refused = add_file(
        &mut dev,
        SPAN,
        "trop.iso",
        100 * MIB,
        false,
        &mut reader,
        &mut |_| true,
    );
    assert!(matches!(refused, Err(FormatError::TooLarge(_))));
    assert_eq!(dev.as_bytes(), before.as_slice());
    Ok(())
}

#[test]
fn un_fichier_retire_libere_sa_place() -> Result<(), FormatError> {
    let mut dev = empty()?;
    let big = vec![1u8; 40 * MIB as usize];
    add(&mut dev, "a.iso", &big)?;
    assert!(matches!(
        add(&mut dev, "b.iso", &big),
        Err(FormatError::TooLarge(_))
    ));
    assert_eq!(remove_file(&mut dev, SPAN, "A.ISO")?, big.len() as u64);
    assert!(listed(&dev)?.is_empty());
    add(&mut dev, "b.iso", &big)?;
    assert_eq!(listed(&dev)?, [("b.iso".to_owned(), big.len() as u64)]);
    assert!(matches!(
        remove_file(&mut dev, SPAN, "absent.iso"),
        Err(FormatError::Name(_))
    ));
    Ok(())
}
