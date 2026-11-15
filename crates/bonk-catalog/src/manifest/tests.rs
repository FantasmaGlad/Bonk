use std::fs;
use std::path::PathBuf;

use ring::digest::{SHA256, digest};

use super::load::*;
use super::verify::*;
use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

const NOW: u64 = 1_000;

fn pair(seed: u8) -> Result<(Ed25519KeyPair, [u8; 32]), Box<dyn std::error::Error>> {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).map_err(|e| e.to_string())?;
    let public = pair.public_key().as_ref().try_into()?;
    Ok((pair, public))
}

fn manifest(sequence: u64, expires: u64, images: &str) -> Vec<u8> {
    format!(
        r#"{{"schema_version":2,"sequence":{sequence},"expires_unix":{expires},"families":[{}],"images":[{images}]}}"#,
        familles()
    )
    .into_bytes()
}

fn familles() -> String {
    let empreinte = "b".repeat(64);
    ["debian", "windows"]
        .map(|id| {
            format!(r#"{{"id":"{id}","name":"{id}","icon":"icons/{id}.svg","icon_sha256":"{empreinte}"}}"#)
        })
        .join(",")
}

fn debian() -> String {
    format!(
        r#"{{"id":"debian-13-netinst-x86_64","family":"debian","name":"Debian","version":"13.6","channel":"stable","released":"2026-01-10","edition":"netinst","arch":"x86_64","homepage":"https://www.debian.org/","url":"https://cdimage.debian.org/x.iso","sha256":"{}","size_bytes":123}}"#,
        "a".repeat(64)
    )
}

/// An identifier a link could not name is refused in the catalog too.
#[test]
fn un_identifiant_hors_de_la_regle_des_liens_est_refuse() -> Result<(), serde_json::Error> {
    for id in ["-debian", &"a".repeat(129)] {
        let image: Image = serde_json::from_str(&debian().replace("debian-13-netinst-x86_64", id))?;
        assert_eq!(
            check_image(&image),
            Err("identifier must be lowercase ASCII"),
            "{id}"
        );
    }
    let image: Image = serde_json::from_str(&debian())?;
    assert_eq!(check_image(&image), Ok(()));
    Ok(())
}

fn windows() -> String {
    r#"{"id":"windows-11-x86_64","family":"windows","name":"Windows","version":"11","channel":"stable","released":"2021-10-05","arch":"x86_64","homepage":"https://www.microsoft.com/","official_page":"https://www.microsoft.com/software-download/windows11"}"#.to_owned()
}

fn check(
    bytes: &[u8],
    key: &Ed25519KeyPair,
    keys: &[[u8; 32]],
    seen: u64,
) -> Result<Catalog, CatalogError> {
    let signature = key.sign(bytes);
    verify(
        bytes,
        signature.as_ref(),
        &Trust {
            keys,
            now_unix: NOW,
            seen_sequence: seen,
        },
    )
}

#[test]
fn un_manifeste_signe_se_lit_avec_ses_deux_formes_d_image() -> Result<(), Box<dyn std::error::Error>>
{
    let (key, public) = pair(1)?;
    let images = format!("{},{}", debian(), windows());
    let catalog = check(&manifest(5, NOW + 1, &images), &key, &[public], 5)?;
    assert_eq!(catalog.images.len(), 2);
    assert!(matches!(
        catalog.images[0].source,
        ImageSource::Download {
            size_bytes: 123,
            ..
        }
    ));
    assert!(matches!(
        catalog.images[1].source,
        ImageSource::OfficialPage { .. }
    ));
    Ok(())
}

#[test]
fn signature_fausse_octet_change_ou_autre_cle_sont_refuses()
-> Result<(), Box<dyn std::error::Error>> {
    let ((key, public), (autre, secours)) = (pair(1)?, pair(2)?);
    let bytes = manifest(1, NOW + 1, &debian());
    assert_eq!(
        check(&bytes, &autre, &[public], 0),
        Err(CatalogError::BadSignature)
    );
    assert!(check(&bytes, &autre, &[public, secours], 0).is_ok());

    let signature = key.sign(&bytes);
    let mut change = bytes.clone();
    change[20] ^= 1;
    let trust = Trust {
        keys: &[public],
        now_unix: NOW,
        seen_sequence: 0,
    };
    assert_eq!(
        verify(&change, signature.as_ref(), &trust),
        Err(CatalogError::BadSignature)
    );
    assert_eq!(verify(&bytes, &[], &trust), Err(CatalogError::BadSignature));
    // Without a key nothing is ever allowed.
    assert_eq!(check(&bytes, &key, &[], 0), Err(CatalogError::BadSignature));
    Ok(())
}

#[test]
fn expire_ancien_ou_format_futur_sont_refuses_meme_signes() -> Result<(), Box<dyn std::error::Error>>
{
    let (key, public) = pair(1)?;
    assert!(matches!(
        check(&manifest(1, NOW, &debian()), &key, &[public], 0),
        Err(CatalogError::Expired { .. })
    ));
    assert!(matches!(
        check(&manifest(4, NOW + 1, &debian()), &key, &[public], 5),
        Err(CatalogError::Rollback {
            sequence: 4,
            seen: 5
        })
    ));
    let futur = br#"{"schema_version":3,"autre_chose":true}"#;
    assert_eq!(
        check(futur, &key, &[public], 0),
        Err(CatalogError::UnsupportedVersion(3))
    );
    assert!(matches!(
        check(b"pas du json", &key, &[public], 0),
        Err(CatalogError::Unreadable(_))
    ));
    Ok(())
}

struct Dossier(PathBuf);

impl Dossier {
    fn new(nom: &str) -> Self {
        Self(std::env::temp_dir().join(format!("bonk-catalog-{nom}-{}", std::process::id())))
    }
}

impl Drop for Dossier {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn la_copie_sert_si_le_reseau_manque_jamais_si_le_fichier_est_refuse()
-> Result<(), Box<dyn std::error::Error>> {
    let (key, public) = pair(1)?;
    let dossier = Dossier::new("copie");
    let bytes = manifest(5, NOW + 1, &debian());
    let signature = key.sign(&bytes).as_ref().to_vec();
    let (m, s) = (bytes.clone(), signature.clone());
    let bon = move |url: &str| {
        Ok(if url.ends_with(".sig") {
            s.clone()
        } else {
            m.clone()
        })
    };
    let hors_ligne = |_: &str| Err("coupure".to_owned());
    let lire = |now, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>| {
        load("https://c/", &dossier.0, now, &[public], fetch)
    };

    assert!(matches!(
        lire(NOW, &hors_ligne),
        Err(CatalogError::Unavailable(_))
    ));
    assert!(!lire(NOW, &bon)?.saved_copy);
    assert!(lire(NOW, &hors_ligne)?.saved_copy);

    // Received but tampered: refused, even with a good copy.
    let (mut f, s2) = (bytes.clone(), signature.clone());
    f[20] ^= 1;
    let falsifie = move |url: &str| {
        Ok(if url.ends_with(".sig") {
            s2.clone()
        } else {
            f.clone()
        })
    };
    assert_eq!(lire(NOW, &falsifie), Err(CatalogError::BadSignature));

    // An older manifest, even well signed, does not pass after a newer one.
    let ancien = manifest(4, NOW + 1, &debian());
    let s3 = key.sign(&ancien).as_ref().to_vec();
    let retour = move |url: &str| {
        Ok(if url.ends_with(".sig") {
            s3.clone()
        } else {
            ancien.clone()
        })
    };
    assert!(matches!(
        lire(NOW, &retour),
        Err(CatalogError::Rollback { .. })
    ));

    // The copy expires too.
    assert!(matches!(
        lire(NOW + 1, &hors_ligne),
        Err(CatalogError::Expired { .. })
    ));
    Ok(())
}

#[test]
fn un_logo_faux_est_ecarte() -> Result<(), Box<dyn std::error::Error>> {
    let dossier = Dossier::new("logo");
    let contenu = b"<svg/>".to_vec();
    let empreinte = hex(digest(&SHA256, &contenu).as_ref());
    let family = |sha: String| Family {
        id: "ubuntu".to_owned(),
        name: "Ubuntu".to_owned(),
        category: None,
        icon: "icons/ubuntu.svg".to_owned(),
        icon_sha256: sha,
        parent: None,
        homepage: None,
        icon_source: None,
        uses: Vec::new(),
        ram_mb: None,
        ram_source: None,
    };
    let fetch = move |_: &str| Ok(contenu.clone());
    assert_eq!(
        icon_path("https://c/", &dossier.0, &family("0".repeat(64)), &fetch),
        None
    );
    let chemin = icon_path("https://c/", &dossier.0, &family(empreinte.clone()), &fetch);
    assert!(chemin.is_some_and(|p| p.is_file()));
    // Already saved: no network.
    let hors_ligne = |_: &str| Err("coupure".to_owned());
    let chemin = icon_path(
        "https://c/",
        &dossier.0,
        &family(empreinte.clone()),
        &hors_ligne,
    );
    // A cached copy changed on disk is not served, even offline.
    std::fs::write(chemin.ok_or("en cache")?, b"<svg>autre</svg>")?;
    assert_eq!(
        icon_path(
            "https://c/",
            &dossier.0,
            &family(empreinte.clone()),
            &hors_ligne
        ),
        None
    );
    let reprise = icon_path("https://c/", &dossier.0, &family(empreinte), &fetch);
    assert_eq!(std::fs::read(reprise.ok_or("retéléchargé")?)?, b"<svg/>");
    Ok(())
}

fn avec_grub(grub: &str) -> Vec<u8> {
    let mut bytes = manifest(1, NOW + 1, &debian());
    bytes.truncate(bytes.len() - 1);
    bytes.extend(format!(r#","grub":{grub}}}"#).into_bytes());
    bytes
}

fn grub(efi_path: &str) -> String {
    let (a, b) = ("a".repeat(64), "b".repeat(64));
    format!(
        r#"{{"version":"2.14","released":"2026-01-14","build":"6e330c761ee82877","efi":{{"x86_64":{{"path":"{efi_path}","sha256":"{a}","size_bytes":1}}}},"source":{{"path":"grub/2.14/grub-2.14.tar.xz","sha256":"{b}","size_bytes":2}},"source_signature":{{"path":"grub/2.14/grub-2.14.tar.xz.sig","sha256":"{b}","size_bytes":3}}}}"#
    )
}

#[test]
fn l_entree_grub_se_lit_et_reste_dans_le_dossier_du_catalogue()
-> Result<(), Box<dyn std::error::Error>> {
    let (key, public) = pair(1)?;
    let catalog = check(&manifest(1, NOW + 1, &debian()), &key, &[public], 0)?;
    assert_eq!(catalog.grub, None);
    let catalog = check(
        &avec_grub(&grub("grub/2.14/BOOTX64.EFI")),
        &key,
        &[public],
        0,
    )?;
    let entry = catalog.grub.ok_or("pas d'entrée grub")?;
    assert_eq!(entry.efi["x86_64"].path, "grub/2.14/BOOTX64.EFI");
    for chemin in [
        "../BOOTX64.EFI",
        "/BOOTX64.EFI",
        "https://autre/BOOTX64.EFI",
    ] {
        assert!(matches!(
            check(&avec_grub(&grub(chemin)), &key, &[public], 0),
            Err(CatalogError::InvalidGrub(_))
        ));
    }
    Ok(())
}

#[test]
fn un_fichier_publie_est_controle_puis_garde_en_cache() -> Result<(), String> {
    let dossier = Dossier::new("asset");
    let contenu = b"MZ grub".to_vec();
    let asset = Asset {
        path: "grub/2.14/BOOTX64.EFI".to_owned(),
        sha256: hex(digest(&SHA256, &contenu).as_ref()),
        size_bytes: 7,
    };
    let faux = |_: &str, _: u64| Ok(b"autre".to_vec());
    assert!(asset_path("https://c/", &dossier.0, "grub", &asset, &faux).is_err());
    let servi = |url: &str, limite: u64| {
        assert_eq!(url, "https://c/grub/2.14/BOOTX64.EFI");
        assert_eq!(limite, 7);
        Ok(contenu.clone())
    };
    let chemin = asset_path("https://c/", &dossier.0, "grub", &asset, &servi)?;
    assert_eq!(fs::read(&chemin).map_err(|e| e.to_string())?, contenu);
    let hors_ligne = |_: &str, _: u64| Err("coupure".to_owned());
    assert_eq!(
        asset_path("https://c/", &dossier.0, "grub", &asset, &hors_ligne)?,
        chemin
    );
    // A cached copy changed on disk is not trusted.
    fs::write(&chemin, b"abime").map_err(|e| e.to_string())?;
    assert!(asset_path("https://c/", &dossier.0, "grub", &asset, &hors_ligne).is_err());
    Ok(())
}

#[test]
fn les_cles_embarquees_sont_distinctes_et_refusent_une_signature_de_test()
-> Result<(), Box<dyn std::error::Error>> {
    assert_ne!(TRUSTED_KEYS[0], TRUSTED_KEYS[1]);
    let (key, _) = pair(1)?;
    assert_eq!(
        check(&manifest(1, NOW + 1, &debian()), &key, &TRUSTED_KEYS, 0),
        Err(CatalogError::BadSignature)
    );
    Ok(())
}

#[test]
fn plusieurs_versions_d_un_systeme_et_cycle_de_vie() -> Result<(), Box<dyn std::error::Error>> {
    let (key, public) = pair(1)?;
    let ancienne = debian()
        .replace("debian-13-netinst", "debian-12-netinst")
        .replace("13.6", "12.11")
        .replace(r#""channel":"stable""#, r#""channel":"end_of_life""#);
    let images = format!("{},{ancienne}", debian());
    let catalog = check(&manifest(1, NOW + 1, &images), &key, &[public], 0)?;
    assert_eq!(catalog.images[1].channel, Channel::EndOfLife);
    assert_eq!(catalog.images[1].family, catalog.images[0].family);
    Ok(())
}

#[test]
fn un_miroir_est_https_ou_onion() {
    assert!(is_mirror("https://m/x.iso"));
    assert!(is_mirror("http://abc.onion/x.iso"));
    assert!(!is_mirror("http://m/x.iso"));
    assert!(!is_mirror("http://onion.evil/abc.onion"));
    assert!(!is_mirror("ftp://abc.onion/x.iso"));
}

#[test]
fn un_logo_ne_sort_pas_du_dossier_du_catalogue() {
    let family = |icon: &str| Family {
        id: "ubuntu".to_owned(),
        name: "Ubuntu".to_owned(),
        category: None,
        icon: icon.to_owned(),
        icon_sha256: "b".repeat(64),
        parent: None,
        homepage: None,
        icon_source: None,
        uses: Vec::new(),
        ram_mb: None,
        ram_source: None,
    };
    assert_eq!(check_family(&family("icons/ubuntu.svg")), Ok(()));
    for refuse in [
        "../x.svg",
        "/x.svg",
        "https://x/y.svg",
        "a//b.svg",
        r"a\b.svg",
        "",
    ] {
        assert!(check_family(&family(refuse)).is_err(), "{refuse}");
    }
}

#[test]
fn une_image_invalide_refuse_tout_le_manifeste() -> Result<(), Box<dyn std::error::Error>> {
    let (key, public) = pair(1)?;
    let cas = [
        debian().replace("https://cdimage", "http://cdimage"),
        debian().replace(&"a".repeat(64), "abc"),
        debian().replace("debian-13", "Debian 13"),
        debian().replace("https://www.debian.org/", "http://www.debian.org/"),
        windows().replace("https://www.microsoft.com/software", "ftp://x/software"),
        format!("{},{}", debian(), debian()),
        debian().replace(r#""family":"debian""#, r#""family":"absente""#),
        debian().replace("2026-01-10", "10 janvier"),
    ];
    for images in cas {
        assert!(
            matches!(
                check(&manifest(1, NOW + 1, &images), &key, &[public], 0),
                Err(CatalogError::InvalidImage { .. })
            ),
            "{images}"
        );
    }
    Ok(())
}
