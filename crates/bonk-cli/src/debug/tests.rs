use std::io;

use super::hexdump::hexdump_lines;
use super::*;

#[test]
fn nombres_decimaux_et_hexadecimaux() {
    assert_eq!(parse_number("512"), Ok(512));
    assert_eq!(parse_number("0x1BE"), Ok(0x1BE));
    assert_eq!(parse_number("0X10"), Ok(16));
    assert!(parse_number("abc").is_err());
    assert!(parse_number("0x").is_err());
    assert!(parse_number("-1").is_err());
}

/// Image de test : MBR avec une partition Linux, dans un fichier temporaire supprimé à la fin.
struct TempImage(PathBuf);

impl TempImage {
    fn mbr(label: &str) -> Result<Self, io::Error> {
        let path =
            std::env::temp_dir().join(format!("bonk-debug-{}-{label}.img", std::process::id()));
        let mut img = vec![0u8; 4096 * 512];
        img[510] = 0x55;
        img[511] = 0xAA;
        img[446 + 4] = 0x83;
        img[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
        img[446 + 12..446 + 16].copy_from_slice(&1000u32.to_le_bytes());
        std::fs::write(&path, img)?;
        Ok(Self(path))
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn texte(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn layout_d_une_image_mbr() -> Result<(), Box<dyn std::error::Error>> {
    let image = TempImage::mbr("layout")?;
    let mut out = Vec::new();
    let args = LayoutArgs {
        image: image.0.clone(),
        json: false,
    };
    assert!(matches!(layout(&args, &mut out)?, Outcome::Done));
    let text = texte(out);
    assert!(
        text.contains("MBR : signature 55 AA oui, protection GPT : non"),
        "{text}"
    );
    assert!(
        text.contains("entrée 1 : type 0x83  début 2048  1000 secteurs"),
        "{text}"
    );
    assert!(text.contains("GPT : aucun"), "{text}");
    assert!(text.contains("Constats : aucun"), "{text}");

    let mut json = Vec::new();
    let args = LayoutArgs {
        image: image.0.clone(),
        json: true,
    };
    layout(&args, &mut json)?;
    let value: serde_json::Value = serde_json::from_slice(&json)?;
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["mbr"]["entries"][0]["start_lba"], 2048);
    assert_eq!(value["logical_block_size"], 512);
    Ok(())
}

#[test]
fn les_commandes_debug_ne_modifient_jamais_l_image() -> Result<(), Box<dyn std::error::Error>> {
    let image = TempImage::mbr("lecture-seule")?;
    let before = std::fs::read(&image.0)?;
    let mtime = std::fs::metadata(&image.0)?.modified()?;
    let mut sink = Vec::new();
    layout(
        &LayoutArgs {
            image: image.0.clone(),
            json: false,
        },
        &mut sink,
    )?;
    layout(
        &LayoutArgs {
            image: image.0.clone(),
            json: true,
        },
        &mut sink,
    )?;
    hexdump(
        &HexdumpArgs {
            image: image.0.clone(),
            offset: 0,
            len: 4096,
        },
        &mut sink,
    )?;
    assert_eq!(std::fs::read(&image.0)?, before);
    assert_eq!(std::fs::metadata(&image.0)?.modified()?, mtime);
    Ok(())
}

#[test]
fn hexdump_borne_et_signale_la_fin_de_l_image() -> Result<(), Box<dyn std::error::Error>> {
    let image = TempImage::mbr("hexdump")?;
    let hex = |offset, len| -> Result<(bool, String), io::Error> {
        let mut out = Vec::new();
        let done = matches!(
            hexdump(
                &HexdumpArgs {
                    image: image.0.clone(),
                    offset,
                    len
                },
                &mut out
            )?,
            Outcome::Done
        );
        Ok((done, texte(out)))
    };
    let (done, text) = hex(0x1c0, 32)?;
    assert!(done && text.starts_with("000001c0  00 00 83"), "{text}");
    // Un décalage non aligné sur le secteur est servi tel quel.
    let (_, text) = hex(0x1fe, 2)?;
    assert!(text.contains("55 aa"), "{text}");
    // Au-delà de la fin : la lecture est ramenée à l'image et le dit.
    let (done, text) = hex(4096 * 512 - 16, 64)?;
    assert!(
        done && text.contains("(fin de l'image : 16 octets lus sur 64 demandés)"),
        "{text}"
    );
    // Décalage hors de l'image et longueurs interdites.
    assert!(!hex(4096 * 512, 16)?.0);
    assert!(!hex(0, 0)?.0);
    assert!(!hex(0, HEXDUMP_MAX + 1)?.0);
    Ok(())
}

#[test]
fn un_disque_reel_est_refuse_avec_une_explication() -> Result<(), io::Error> {
    let mut out = Vec::new();
    let args = LayoutArgs {
        image: PathBuf::from("/dev/disque-qui-n-existe-pas"),
        json: false,
    };
    match layout(&args, &mut out)? {
        Outcome::Failed(code, message) => {
            assert_eq!(code, Code::RawDiskUnavailable);
            assert!(message.contains("n'existe pas"), "{message}");
        }
        Outcome::Done => panic!("un disque réel ne doit pas être lu"),
    }
    assert!(out.is_empty());
    // Un répertoire ou un fichier spécial n'est pas non plus ouvert (FileDevice les refuse).
    let args = LayoutArgs {
        image: std::env::temp_dir(),
        json: false,
    };
    assert!(matches!(
        layout(&args, &mut Vec::new())?,
        Outcome::Failed(Code::SourceUnreadable, _)
    ));
    Ok(())
}

#[test]
fn lignes_de_hexdump() {
    let lines = hexdump_lines(0x1b0, b"EFI PART\x00\x01\x02ABCDEFG");
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0],
        "000001b0  45 46 49 20 50 41 52 54  00 01 02 41 42 43 44 45  |EFI PART...ABCDE|"
    );
    // Ligne incomplète : complétée par des blancs pour garder le texte aligné.
    assert_eq!(
        lines[1],
        "000001c0  46 47                                             |FG|"
    );
    assert!(hexdump_lines(0, &[]).is_empty());
}
