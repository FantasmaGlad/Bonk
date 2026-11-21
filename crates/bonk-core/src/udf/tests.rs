#![allow(clippy::unwrap_used)]

use super::*;

fn names(tree: &Tree) -> Vec<String> {
    tree.files.iter().map(|f| f.path.join("/")).collect()
}

#[test]
fn l_arbre_se_lit_avec_ses_trois_formes_de_contenu() {
    let image = testing::image();
    let udf = Udf::open(image.as_slice()).unwrap();
    let tree = udf.tree().unwrap();
    assert_eq!(names(&tree), ["SOURCES/boot.wim", "readme.txt"]);
    assert_eq!(
        tree.directories,
        [vec!["SOURCES".to_owned()], vec!["vide".to_owned()]]
    );

    let inline = tree.find(&["README.TXT"]).unwrap();
    assert_eq!(inline.size, 5);
    let mut buf = [0; 5];
    read_content(image.as_slice(), &inline.content, 0, &mut buf).unwrap();
    assert_eq!(&buf, b"salut");

    let parts = tree.find(&["sources", "boot.wim"]).unwrap();
    assert_eq!(parts.size, 3000);
    let mut buf = vec![0; 3000];
    read_content(image.as_slice(), &parts.content, 0, &mut buf).unwrap();
    assert!(buf[..2048].iter().all(|b| *b == 0xAA));
    assert!(buf[2048..].iter().all(|b| *b == 0xBB));
    let mut buf = [0; 4];
    read_content(image.as_slice(), &parts.content, 2046, &mut buf).unwrap();
    assert_eq!(buf, [0xAA, 0xAA, 0xBB, 0xBB]);
}

#[test]
fn sans_ancre_ce_n_est_pas_de_l_udf() {
    let image = vec![0u8; 1 << 20];
    assert!(matches!(Udf::open(image.as_slice()), Err(UdfError::NotUdf)));
    assert!(matches!(Udf::open([].as_slice()), Err(UdfError::NotUdf)));
}

#[test]
fn un_nom_ne_peut_pas_sortir_de_l_arbre() {
    for raw in [
        &b"\x08.."[..],
        b"\x08a/b",
        b"\x08a\\b",
        b"\x08",
        b"",
        b"\x07abc",
        b"\x10abc",
    ] {
        assert!(decode_name(raw).is_err(), "{raw:?}");
    }
    assert_eq!(decode_name(b"\x08caf\xE9").unwrap(), "café");
    assert_eq!(decode_name(b"\x10\x00a\x00b").unwrap(), "ab");
}

#[test]
fn une_image_abimee_ne_fait_jamais_paniquer() {
    let good = testing::image();
    let ranges = [
        256 * 2048..256 * 2048 + 32,
        258 * 2048 + 200..258 * 2048 + 460,
        300 * 2048..309 * 2048 + 240,
    ];
    for range in ranges {
        for at in range.step_by(3) {
            let mut bad = good.clone();
            bad[at] ^= 0xFF;
            if let Ok(udf) = Udf::open(bad.as_slice()) {
                let _ = udf.tree();
            }
        }
    }
}
