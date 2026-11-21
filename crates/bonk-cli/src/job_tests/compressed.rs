use super::*;

fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
    let mut chunks = data.chunks(0xFFFF).peekable();
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(chunk);
    }
    out.extend(bonk_core::partition::crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

fn cut_iso(sectors: u32) -> Vec<u8> {
    let mut image: Vec<u8> = (0..1usize << 20).map(|i| (i % 251) as u8).collect();
    image[446..510].fill(0);
    image[510] = 0x55;
    image[511] = 0xAA;
    let primary = 16 * 2048;
    image[primary] = 1;
    image[primary + 1..primary + 6].copy_from_slice(b"CD001");
    image[primary + 80..primary + 84].copy_from_slice(&sectors.to_le_bytes());
    image[primary + 128..primary + 130].copy_from_slice(&2048u16.to_le_bytes());
    image
}

#[test]
fn image_compressee_coupee_a_l_interieur_n_aboutit_pas_a_un_succes() {
    let source = TempSource::from_bytes("coupee-gz", &gzip_stored(&cut_iso(2048)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3007"), "{err}");
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn image_compressee_annoncee_plus_grande_que_la_cible_ne_touche_a_rien() {
    let source = TempSource::from_bytes("grande-gz", &gzip_stored(&cut_iso(100_000)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3003"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, EXIT_SOURCE);
}

#[test]
fn image_vide_refusee_meme_en_mode_brut() {
    for source in [
        TempSource::from_bytes("vide", &[]),
        TempSource::from_bytes("vide-gz", &gzip_stored(&[])),
    ] {
        let scenario = Scenario::new();
        let (code, _, err) =
            scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
        assert_eq!(code, EXIT_SOURCE, "{err}");
        assert!(scenario.log().is_empty());
    }
}

/// Longer than the head the analysis reads: only a full pass sees its end.
const LONG_IMAGE: usize = 6 << 20;

#[test]
fn des_donnees_apres_le_flux_sont_refusees_avant_toute_ecriture() {
    let mut image = gzip_stored(&vec![0xAB; LONG_IMAGE]);
    image.extend_from_slice(&[1; 4096]);
    let source = TempSource::from_bytes("flux-suivi", &image);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{out}\n{err}");
    assert!(err.contains("E3004"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(4096).iter().all(|b| *b == 0));
}

#[test]
fn une_image_compressee_saine_s_ecrit_apres_le_passage_a_blanc() {
    let source = TempSource::from_bytes("flux-sain", &gzip_stored(&vec![0xAB; LONG_IMAGE]));
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(scenario.written(LONG_IMAGE).iter().all(|b| *b == 0xAB));
}
