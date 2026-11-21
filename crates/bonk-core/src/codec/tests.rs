use std::io::Cursor;

use super::testing::{gzip, xz_stored, xz_stored_with_dictionary};
use super::*;

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

fn read_all(source: &mut Source) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    source.reader.read_to_end(&mut out)?;
    Ok(out)
}

#[test]
fn formats_reconnus_par_leurs_premiers_octets() {
    assert_eq!(
        Format::detect(&[0xFD, b'7', b'z', b'X', b'Z', 0, 1]),
        Format::Xz
    );
    assert_eq!(Format::detect(&[0x28, 0xB5, 0x2F, 0xFD, 0]), Format::Zstd);
    assert_eq!(Format::detect(&[0x1F, 0x8B, 8]), Format::Gzip);
    assert_eq!(Format::detect(b"BZh91AY"), Format::Bzip2);
    assert_eq!(Format::detect(b"PK\x03\x04"), Format::Zip);
    assert_eq!(Format::detect(b"\xEB\x63\x90MSDOS"), Format::Raw);
    assert_eq!(Format::detect(&[]), Format::Raw);
    assert_eq!(Format::detect(&[0x1F]), Format::Raw);
}

#[test]
fn image_brute_lue_telle_quelle_avec_sa_taille() -> Result<(), Box<dyn std::error::Error>> {
    let data = sample(10_000);
    let mut source = open(Cursor::new(data.clone()), Some(data.len() as u64))?;
    assert_eq!(source.format, Format::Raw);
    assert_eq!(source.output_len, Some(10_000));
    assert_eq!(read_all(&mut source)?, data);
    Ok(())
}

#[test]
fn gzip_decompresse_en_flux_et_compte_l_entree() -> Result<(), Box<dyn std::error::Error>> {
    let data = sample(300_000);
    let packed = gzip(&data);
    let packed_len = packed.len() as u64;
    let mut source = open(Cursor::new(packed), Some(packed_len))?;
    assert_eq!(source.format, Format::Gzip);
    assert_eq!(source.output_len, None);
    assert_eq!(read_all(&mut source)?, data);
    Ok(())
}

#[test]
fn gzip_concatene_est_lu_en_entier() -> Result<(), Box<dyn std::error::Error>> {
    let (a, b) = (sample(1000), sample(2000));
    let mut packed = gzip(&a);
    packed.extend(gzip(&b));
    let mut source = open(Cursor::new(packed), None)?;
    let mut expected = a;
    expected.extend(b);
    assert_eq!(read_all(&mut source)?, expected);
    Ok(())
}

#[test]
fn xz_decompresse_en_flux() -> Result<(), Box<dyn std::error::Error>> {
    let data = sample(200_000);
    let mut source = open(Cursor::new(xz_stored(&data)), None)?;
    assert_eq!(source.format, Format::Xz);
    assert_eq!(read_all(&mut source)?, data);
    Ok(())
}

#[test]
fn xz_au_dictionnaire_demesure_est_refuse_sans_allocation() -> Result<(), Box<dyn std::error::Error>>
{
    let data = sample(1000);
    let mut source = open(Cursor::new(xz_stored_with_dictionary(&data, 30)), None)?;
    assert_eq!(read_all(&mut source)?, data);
    let mut source = open(Cursor::new(xz_stored_with_dictionary(&data, 40)), None)?;
    assert!(read_all(&mut source).is_err());
    Ok(())
}

#[test]
fn xz_tronque_donne_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
    let data = sample(200_000);
    let mut packed = xz_stored(&data);
    packed.truncate(packed.len() - 40);
    let mut source = open(Cursor::new(packed), None)?;
    let mut out = Vec::new();
    assert!(source.reader.read_to_end(&mut out).is_err());
    assert!(data.starts_with(&out));
    Ok(())
}

#[test]
fn xz_lecture_partielle_puis_abandon() -> Result<(), Box<dyn std::error::Error>> {
    let data = sample(3_000_000);
    let mut source = open(Cursor::new(xz_stored(&data)), None)?;
    let mut first = [0u8; 100];
    source.reader.read_exact(&mut first)?;
    assert_eq!(&first[..], &data[..100]);
    drop(source);
    Ok(())
}

#[test]
fn gzip_corrompu_est_une_erreur_de_lecture() -> Result<(), Box<dyn std::error::Error>> {
    let mut packed = gzip(&sample(50_000));
    let middle = packed.len() / 2;
    packed[middle] ^= 0xFF;
    packed.truncate(packed.len() - 8);
    let mut source = open(Cursor::new(packed), None)?;
    assert!(read_all(&mut source).is_err());
    Ok(())
}

#[test]
fn bzip2_et_zip_sont_refuses_avec_un_message_utile() {
    for (head, format) in [
        (&b"BZh91AY&SY"[..], Format::Bzip2),
        (&b"PK\x03\x04rest"[..], Format::Zip),
    ] {
        let error = open(Cursor::new(head.to_vec()), None).err();
        assert!(matches!(error, Some(CodecError::Unsupported(f)) if f == format));
        assert!(error.is_some_and(|e| e.to_string().contains("decompress the image")));
    }
}

fn zstd(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

#[test]
fn zstd_a_plusieurs_trames_est_lu_en_entier() -> Result<(), Box<dyn std::error::Error>> {
    let (a, b) = (sample(100_000), sample(70_000));
    let mut packed = zstd(&a);
    packed.extend(zstd(&b));
    let mut source = open(Cursor::new(packed), None)?;
    let mut expected = a;
    expected.extend(b);
    assert_eq!(read_all(&mut source)?, expected);
    Ok(())
}

#[test]
fn zstd_saute_une_trame_ignorable() -> Result<(), Box<dyn std::error::Error>> {
    let (a, b) = (sample(1000), sample(2000));
    let mut packed = zstd(&a);
    packed.extend(0x184D_2A50u32.to_le_bytes());
    packed.extend(5u32.to_le_bytes());
    packed.extend(b"xxxxx");
    packed.extend(zstd(&b));
    let mut source = open(Cursor::new(packed), None)?;
    let mut expected = a;
    expected.extend(b);
    assert_eq!(read_all(&mut source)?, expected);
    Ok(())
}

#[test]
fn zstd_suivi_de_donnees_est_une_erreur_comme_gzip_et_xz() -> Result<(), Box<dyn std::error::Error>>
{
    let mut packed = zstd(&sample(1000));
    packed.extend(b"GARBAGE-GARBAGE");
    let mut source = open(Cursor::new(packed), None)?;
    assert!(read_all(&mut source).is_err());
    Ok(())
}

#[test]
fn zstd_a_somme_fausse_est_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
    let data = b"bonk";
    let mut packed = vec![0x28, 0xB5, 0x2F, 0xFD, 0x24, data.len() as u8];
    packed.extend([(data.len() as u8) << 3 | 1, 0, 0]);
    packed.extend(data);
    packed.extend([0, 0, 0, 0]);
    let mut source = open(Cursor::new(packed), None)?;
    assert!(read_all(&mut source).is_err());
    Ok(())
}

#[test]
fn zstd_tronque_est_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
    let mut packed = zstd(&sample(300_000));
    packed.truncate(packed.len() * 3 / 4);
    let mut source = open(Cursor::new(packed), None)?;
    assert!(read_all(&mut source).is_err());
    Ok(())
}

#[test]
fn zstd_signature_invalide_apres_le_magic_est_une_erreur() {
    let error = open(
        Cursor::new(vec![0x28, 0xB5, 0x2F, 0xFD, 0xFF, 0xFF, 0xFF]),
        None,
    )
    .err();
    assert!(matches!(error, Some(CodecError::Io(_))));
}
