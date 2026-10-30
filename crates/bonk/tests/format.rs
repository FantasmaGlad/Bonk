//! `bonk format --simulate`, relu par les outils de référence : `sgdisk` pour la table, `fsck.vfat` pour le
//! système de fichiers. Aucun disque réel.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{assert_code, diagnostic, outil_disponible};

struct Image(PathBuf);

impl Image {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("bonk-format-{}-{label}.img", std::process::id())))
    }
    fn arg(&self) -> &str {
        self.0.to_str().unwrap_or_default()
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn bonk(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("bonk : {e}"))
}

fn tool(name: &str, args: &[&str]) -> std::process::Output {
    Command::new(name)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{name} : {e}"))
}

/// Découpe la partition dans un fichier à part : `fsck.vfat` ne connaît pas les décalages.
fn extract(image: &Path, first_lba: u64, sectors: u64, out: &Path) {
    let data = std::fs::read(image).unwrap_or_else(|e| panic!("{e}"));
    let start = usize::try_from(first_lba * 512).unwrap_or(0);
    let end = start + usize::try_from(sectors * 512).unwrap_or(0);
    std::fs::write(out, &data[start..end]).unwrap_or_else(|e| panic!("{e}"));
}

fn format_json(image: &Image, size: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "format",
        "--simulate",
        image.arg(),
        "--size",
        size,
        "--yes",
        "--json",
    ];
    args.extend_from_slice(extra);
    let sortie = bonk(&args);
    assert_code(&sortie, 0);
    serde_json::from_slice(&sortie.stdout).unwrap_or_else(|e| panic!("{e}"))
}

fn check_fat(image: &Image, value: &serde_json::Value, label: &str) {
    let first = value["partition"]["first_lba"].as_u64().unwrap_or(0);
    let last = value["partition"]["last_lba"].as_u64().unwrap_or(0);
    let part = Image::new(&format!("{label}-partition"));
    extract(&image.0, first, last - first + 1, &part.0);
    let fsck = tool("fsck.vfat", &["-n", part.arg()]);
    assert_eq!(fsck.status.code(), Some(0), "{}", diagnostic(&fsck));
}

#[test]
fn gpt_et_fat32_acceptes_par_sgdisk_et_fsck_vfat() {
    if !outil_disponible("sgdisk") || !outil_disponible("fsck.vfat") {
        return;
    }
    // Sous, à, et au-dessus des seuils de taille de cluster.
    for (size, label) in [("67108864", "64m"), ("536870912", "512m")] {
        let image = Image::new(label);
        let value = format_json(&image, size, &["--label", "bonk"]);
        let check = tool("sgdisk", &["--verify", image.arg()]);
        assert_code(&check, 0);
        let text = String::from_utf8_lossy(&check.stdout);
        assert!(text.contains("No problems found"), "{text}");
        check_fat(&image, &value, label);
    }
}

#[test]
fn mbr_et_fat32_acceptes_par_fsck_vfat() {
    if !outil_disponible("fsck.vfat") {
        return;
    }
    let image = Image::new("mbr");
    let value = format_json(&image, "67108864", &["--table", "mbr"]);
    assert_eq!(value["table"], "mbr");
    check_fat(&image, &value, "mbr");
}

#[test]
fn trop_petit_pour_fat32_ne_cree_pas_de_fichier_ecrit() {
    let image = Image::new("petit");
    let sortie = bonk(&[
        "format",
        "--simulate",
        image.arg(),
        "--size",
        "8388608",
        "--yes",
    ]);
    assert_code(&sortie, 1);
    assert!(
        String::from_utf8_lossy(&sortie.stderr).contains("E4006"),
        "{}",
        diagnostic(&sortie)
    );
    assert!(!image.0.exists());
}

#[test]
fn les_autres_systemes_ne_s_ecrivent_pas_sur_un_fichier_image() {
    let image = Image::new("exfat");
    let sortie = bonk(&[
        "format",
        "--simulate",
        image.arg(),
        "--size",
        "67108864",
        "--fs",
        "exfat",
        "--yes",
    ]);
    assert_code(&sortie, 1);
    assert!(
        String::from_utf8_lossy(&sortie.stderr).contains("E4007"),
        "{}",
        diagnostic(&sortie)
    );
}

#[test]
fn capabilities_en_json_est_valide_sur_tout_systeme() {
    let sortie = bonk(&["capabilities", "--json"]);
    assert_code(&sortie, 0);
    let value: serde_json::Value =
        serde_json::from_slice(&sortie.stdout).unwrap_or_else(|e| panic!("{e}"));
    let names: Vec<&str> = value["capabilities"]
        .as_array()
        .map(|all| all.iter().filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();
    for attendu in [
        "write",
        "elevate",
        "eject",
        "cancel",
        "format.fat32",
        "format.exfat",
        "format.ntfs",
        "format.ext4",
    ] {
        assert!(names.contains(&attendu), "{attendu} : {names:?}");
    }
}
