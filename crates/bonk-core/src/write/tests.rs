use std::io::Cursor;

use super::*;
use crate::block::MemoryDevice;

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 253) as u8).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn silent() -> impl FnMut(&Progress) -> Control {
    |_| Control::Continue
}

#[test]
fn ecriture_puis_verification_d_une_image_multiple_du_secteur()
-> Result<(), Box<dyn std::error::Error>> {
    let image = data(8 * 512);
    let mut dev = MemoryDevice::new(16 * 512, 512)?;
    let report = write_image(
        &mut Cursor::new(image.clone()),
        &mut dev,
        Some(image.len() as u64),
        1024,
        &mut silent(),
    )?;
    assert_eq!((report.bytes, report.padded_bytes), (4096, 4096));
    assert_eq!(report.sha256, sha(&image));
    assert_eq!(&dev.as_bytes()[..4096], image.as_slice());
    assert!(dev.as_bytes()[4096..].iter().all(|&b| b == 0));
    let verified = verify(&mut dev, report.bytes, &report.sha256, 1024, &mut silent())?;
    assert_eq!(verified.sha256, report.sha256);
    Ok(())
}

#[test]
fn la_derniere_partie_est_completee_a_zero_sans_changer_l_empreinte()
-> Result<(), Box<dyn std::error::Error>> {
    let image = data(1000); // not a multiple of 512
    let mut dev = MemoryDevice::new(8 * 512, 512)?;
    let report = write_image(
        &mut Cursor::new(image.clone()),
        &mut dev,
        None,
        4096,
        &mut silent(),
    )?;
    assert_eq!((report.bytes, report.padded_bytes), (1000, 1024));
    assert_eq!(report.sha256, sha(&image));
    assert_eq!(&dev.as_bytes()[..1000], image.as_slice());
    assert!(dev.as_bytes()[1000..1024].iter().all(|&b| b == 0));
    verify(&mut dev, 1000, &report.sha256, 4096, &mut silent())?;
    Ok(())
}

#[test]
fn secteurs_de_4096_octets_et_blocs_plus_petits_que_le_secteur()
-> Result<(), Box<dyn std::error::Error>> {
    let image = data(10_000);
    let mut dev = MemoryDevice::new(8 * 4096, 4096)?;
    let report = write_image(
        &mut Cursor::new(image.clone()),
        &mut dev,
        None,
        100,
        &mut silent(),
    )?;
    assert_eq!((report.bytes, report.padded_bytes), (10_000, 12_288));
    verify(&mut dev, report.bytes, &report.sha256, 100, &mut silent())?;
    Ok(())
}

#[test]
fn image_connue_trop_grande_refusee_sans_rien_ecrire() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(4 * 512, 512)?;
    let image = data(5000);
    let error = write_image(
        &mut Cursor::new(image),
        &mut dev,
        Some(5000),
        512,
        &mut silent(),
    )
    .err();
    assert!(matches!(
        error,
        Some(WriteError::TooLarge {
            device_size: 2048,
            needed: Some(5000)
        })
    ));
    assert!(dev.as_bytes().iter().all(|&b| b == 0));
    Ok(())
}

#[test]
fn image_de_taille_inconnue_qui_deborde_s_arrete_a_la_fin_du_support()
-> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(4 * 512, 512)?;
    let image = data(5000);
    let error = write_image(
        &mut Cursor::new(image.clone()),
        &mut dev,
        None,
        512,
        &mut silent(),
    )
    .err();
    assert!(matches!(
        error,
        Some(WriteError::TooLarge { needed: None, .. })
    ));
    assert_eq!(dev.as_bytes(), &image[..2048]);
    Ok(())
}

#[test]
fn annulation_entre_deux_blocs() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(16 * 512, 512)?;
    let mut calls = 0;
    let mut cancel_after_two = |_: &Progress| {
        calls += 1;
        if calls > 2 {
            Control::Cancel
        } else {
            Control::Continue
        }
    };
    let error = write_image(
        &mut Cursor::new(data(8 * 512)),
        &mut dev,
        None,
        512,
        &mut cancel_after_two,
    )
    .err();
    assert!(matches!(
        error,
        Some(WriteError::Cancelled {
            bytes_written: 1024
        })
    ));
    Ok(())
}

#[test]
fn progression_croissante_avec_phases() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(16 * 512, 512)?;
    let mut seen = Vec::new();
    let mut record = |p: &Progress| {
        seen.push((p.phase, p.bytes_done));
        Control::Continue
    };
    write_image(
        &mut Cursor::new(data(4 * 512)),
        &mut dev,
        Some(2048),
        512,
        &mut record,
    )?;
    assert_eq!(seen.first(), Some(&(Phase::Write, 0)));
    assert_eq!(seen.last(), Some(&(Phase::Flush, 2048)));
    let writes: Vec<u64> = seen
        .iter()
        .filter(|(p, _)| *p == Phase::Write)
        .map(|(_, b)| *b)
        .collect();
    assert!(writes.windows(2).all(|w| w[0] <= w[1]), "{writes:?}");
    assert_eq!(writes.last(), Some(&2048));
    Ok(())
}

#[test]
fn la_verification_detecte_un_octet_different() -> Result<(), Box<dyn std::error::Error>> {
    let image = data(8 * 512);
    let mut dev = MemoryDevice::new(16 * 512, 512)?;
    let report = write_image(&mut Cursor::new(image), &mut dev, None, 1024, &mut silent())?;
    let mut tampered = dev.as_bytes().to_vec();
    tampered[3000] ^= 0x01;
    let mut bad = MemoryDevice::from_bytes(tampered, 512)?;
    let error = verify(&mut bad, report.bytes, &report.sha256, 1024, &mut silent()).err();
    assert!(
        matches!(error, Some(VerifyError::Mismatch { ref expected, ref actual }) if *expected == report.sha256 && actual != expected)
    );
    assert!(
        verify(
            &mut dev,
            report.bytes - 512,
            &report.sha256,
            1024,
            &mut silent()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn hash_source_et_comparaison_sans_ecriture() -> Result<(), Box<dyn std::error::Error>> {
    let image = data(3000);
    let (len, digest) = hash_source_ignoring(
        &mut Cursor::new(image.clone()),
        Some(3000),
        700,
        &[],
        &mut silent(),
    )?;
    assert_eq!((len, digest.as_str()), (3000, sha(&image).as_str()));
    let mut dev = MemoryDevice::new(8 * 512, 512)?;
    write_image(&mut Cursor::new(image), &mut dev, None, 512, &mut silent())?;
    verify(&mut dev, len, &digest, 512, &mut silent())?;
    Ok(())
}

#[test]
fn erreur_de_lecture_de_la_source() -> Result<(), Box<dyn std::error::Error>> {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::InvalidData, "flux corrompu"))
        }
    }
    let mut dev = MemoryDevice::new(4 * 512, 512)?;
    let error = write_image(&mut Broken, &mut dev, None, 512, &mut silent()).err();
    assert!(matches!(error, Some(WriteError::Source(_))));
    Ok(())
}

#[test]
fn la_relecture_dit_si_le_cache_a_ete_vide() -> Result<(), Box<dyn std::error::Error>> {
    struct Sticky(MemoryDevice);
    impl BlockDevice for Sticky {
        fn size(&self) -> u64 {
            self.0.size()
        }
        fn block_size(&self) -> u32 {
            self.0.block_size()
        }
        fn read_at(&self, o: u64, b: &mut [u8]) -> Result<(), BlockError> {
            self.0.read_at(o, b)
        }
        fn write_at(&mut self, o: u64, d: &[u8]) -> Result<(), BlockError> {
            self.0.write_at(o, d)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
        fn drop_cache(&mut self) -> Result<(), BlockError> {
            Err(BlockError::Io(io::Error::other("refusé")))
        }
    }
    let image = data(2048);
    let mut dev = MemoryDevice::new(4096, 512)?;
    let report = write_image(
        &mut Cursor::new(image.clone()),
        &mut dev,
        None,
        512,
        &mut silent(),
    )?;
    assert!(verify(&mut dev, report.bytes, &report.sha256, 512, &mut silent())?.cache_dropped);
    let mut sticky = Sticky(dev);
    let verified = verify(
        &mut sticky,
        report.bytes,
        &report.sha256,
        512,
        &mut silent(),
    )?;
    assert!(!verified.cache_dropped);
    Ok(())
}

#[test]
fn ecriture_sur_un_peripherique_en_lecture_seule_est_une_erreur()
-> Result<(), Box<dyn std::error::Error>> {
    struct ReadOnly(MemoryDevice);
    impl BlockDevice for ReadOnly {
        fn size(&self) -> u64 {
            self.0.size()
        }
        fn block_size(&self) -> u32 {
            self.0.block_size()
        }
        fn read_at(&self, o: u64, b: &mut [u8]) -> Result<(), BlockError> {
            self.0.read_at(o, b)
        }
        fn write_at(&mut self, _: u64, _: &[u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }
    let mut dev = ReadOnly(MemoryDevice::new(4 * 512, 512)?);
    let error = write_image(
        &mut Cursor::new(data(512)),
        &mut dev,
        None,
        512,
        &mut silent(),
    )
    .err();
    assert!(matches!(
        error,
        Some(WriteError::Device(BlockError::ReadOnly))
    ));
    Ok(())
}

#[test]
fn les_plages_ignorees_sont_exclues_des_deux_cotes_et_le_reste_est_compare()
-> Result<(), Box<dyn std::error::Error>> {
    let image: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let ignore = [10..20u64, 4090..4096];
    let (len, sha) = hash_source_ignoring(
        &mut Cursor::new(image.clone()),
        None,
        1024,
        &ignore,
        &mut |_| Control::Continue,
    )?;
    assert_eq!(len, 4096);
    let mut other = image.clone();
    other[12] ^= 0xFF;
    other[4092] ^= 0xFF;
    let mut dev = MemoryDevice::from_bytes(other.clone(), 512)?;
    assert!(
        verify_ignoring(&mut dev, len, &sha, 1024, &ignore, &mut |_| {
            Control::Continue
        })
        .is_ok()
    );
    assert!(verify(&mut dev, len, &sha, 1024, &mut |_| Control::Continue).is_err());
    other[100] ^= 1;
    let mut dev = MemoryDevice::from_bytes(other, 512)?;
    assert!(matches!(
        verify_ignoring(&mut dev, len, &sha, 1024, &ignore, &mut |_| {
            Control::Continue
        }),
        Err(VerifyError::Mismatch { .. })
    ));
    Ok(())
}
