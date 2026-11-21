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
        trace_io: None,
    };
    assert!(matches!(layout(&args, &mut out)?, Outcome::Done));
    let text = texte(out);
    assert!(
        text.contains("MBR: 55 AA signature yes, GPT protection: no"),
        "{text}"
    );
    assert!(
        text.contains("entry 1: type 0x83  start 2048  1000 sectors"),
        "{text}"
    );
    assert!(text.contains("GPT: none"), "{text}");
    assert!(text.contains("Findings: none"), "{text}");

    let mut json = Vec::new();
    let args = LayoutArgs {
        image: image.0.clone(),
        json: true,
        trace_io: None,
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
            trace_io: None,
        },
        &mut sink,
    )?;
    layout(
        &LayoutArgs {
            image: image.0.clone(),
            json: true,
            trace_io: None,
        },
        &mut sink,
    )?;
    hexdump(
        &HexdumpArgs {
            image: image.0.clone(),
            offset: 0,
            len: 4096,
            trace_io: None,
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
                    len,
                    trace_io: None,
                },
                &mut out
            )?,
            Outcome::Done
        );
        Ok((done, texte(out)))
    };
    let (done, text) = hex(0x1c0, 32)?;
    assert!(done && text.starts_with("000001c0  00 00 83"), "{text}");
    let (_, text) = hex(0x1fe, 2)?;
    assert!(text.contains("55 aa"), "{text}");
    let (done, text) = hex(4096 * 512 - 16, 64)?;
    assert!(
        done && text.contains("(end of image: 16 bytes read of 64 requested)"),
        "{text}"
    );
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
        trace_io: None,
    };
    match layout(&args, &mut out)? {
        Outcome::Failed(code, message) => {
            assert_eq!(code, Code::RawDiskUnavailable);
            assert!(message.contains("does not exist"), "{message}");
        }
        Outcome::Done => panic!("un disque réel ne doit pas être lu"),
    }
    assert!(out.is_empty());
    let args = LayoutArgs {
        image: std::env::temp_dir(),
        json: false,
        trace_io: None,
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
    assert_eq!(
        lines[1],
        "000001c0  46 47                                             |FG|"
    );
    assert!(hexdump_lines(0, &[]).is_empty());
}

fn one_disk() -> Vec<bonk_hal::Disk> {
    vec![bonk_hal::Disk {
        id: "usb-cle".to_owned(),
        path: r"\\.\PhysicalDrive2".to_owned(),
        model: "Clé".to_owned(),
        serial: None,
        size_bytes: 1 << 30,
        bus: bonk_hal::Bus::Usb,
        kind: bonk_hal::DiskKind::Physical,
        removable_flag: true,
        logical_block_size: None,
        location: None,
        fs_type: None,
        holders: vec![],
        partitions: vec![],
    }]
}

#[test]
fn un_echec_d_enumeration_donne_son_code_et_sa_raison() {
    let echec = layout::disk_named("usb-cle", || {
        Err(bonk_hal::EnumerateError::Malformed {
            what: "udev".to_owned(),
            detail: "illisible".to_owned(),
        })
    });
    let Err((code, message)) = echec else {
        panic!("l'échec d'énumération est perdu");
    };
    assert_eq!(code, Code::EnumerationFailed);
    assert!(message.contains("illisible"), "{message}");
    let absent = layout::disk_named("x", || {
        Err(bonk_hal::EnumerateError::Unsupported { os: "haiku" })
    });
    assert!(matches!(absent, Err((Code::EnumerationUnsupported, _))));
}

#[test]
fn un_disque_se_nomme_par_son_identifiant_ou_son_chemin() {
    assert!(matches!(
        layout::disk_named("usb-cle", || Ok(one_disk())),
        Ok(Some(_))
    ));
    assert!(matches!(
        layout::disk_named(r"\\.\physicaldrive2", || Ok(one_disk())),
        Ok(Some(_))
    ));
    assert!(matches!(
        layout::disk_named("autre", || Ok(one_disk())),
        Ok(None)
    ));
}
