use std::error::Error;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use super::mounts::unescape_octal;
use super::*;
use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};

/// Arborescence factice supprimée à la fin du test.
struct Fake {
    base: PathBuf,
}

impl Fake {
    fn new() -> Result<Self, Box<dyn Error>> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let base = std::env::temp_dir().join(format!("bonk-enum-{}-{n}", std::process::id()));
        let fake = Self { base };
        fake.put("proc/self/mountinfo", "")?;
        fake.put(
            "proc/swaps",
            "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n",
        )?;
        fs::create_dir_all(fake.base.join("run/udev/data"))?;
        fs::create_dir_all(fake.base.join("sys/block"))?;
        fs::create_dir_all(fake.base.join("dev"))?;
        Ok(fake)
    }

    fn roots(&self) -> LinuxRoots {
        LinuxRoots::under(&self.base)
    }

    fn put(&self, rel: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.base.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    /// Crée un disque sous `sys/<device_path>` et le lien `sys/block/<name>`.
    fn disk(
        &self,
        name: &str,
        device_path: &str,
        devnum: &str,
        sectors: u64,
        extra: &[(&str, &str)],
    ) -> Result<(), Box<dyn Error>> {
        let dir = format!("sys/{device_path}");
        self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
        self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
        self.put(&format!("{dir}/removable"), "0\n")?;
        fs::create_dir_all(self.base.join(&dir).join("holders"))?;
        for (file, content) in extra {
            self.put(&format!("{dir}/{file}"), content)?;
        }
        symlink(
            format!("../{device_path}"),
            self.base.join("sys/block").join(name),
        )?;
        Ok(())
    }

    fn partition(
        &self,
        device_path: &str,
        name: &str,
        number: u32,
        devnum: &str,
        sectors: u64,
    ) -> Result<(), Box<dyn Error>> {
        let dir = format!("sys/{device_path}/{name}");
        self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
        self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
        self.put(&format!("{dir}/partition"), &format!("{number}\n"))?;
        fs::create_dir_all(self.base.join(&dir).join("holders"))?;
        Ok(())
    }

    fn udev(&self, devnum: &str, lines: &str) -> Result<(), Box<dyn Error>> {
        self.put(&format!("run/udev/data/b{devnum}"), lines)
    }

    fn mountinfo(&self, text: &str) -> Result<(), Box<dyn Error>> {
        self.put("proc/self/mountinfo", text)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

const NVME_PATH: &str = "devices/pci0000:00/0000:00:02.4/0000:02:00.0/nvme/nvme0/nvme0n1";
const USB_PATH: &str =
    "devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb";
const SATA_PATH: &str = "devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda";

/// Un poste dont le système est sur NVMe, d'après la structure relevée sur une machine réelle.
fn desktop(fake: &Fake) -> Result<(), Box<dyn Error>> {
    fake.disk(
        "nvme0n1",
        NVME_PATH,
        "259:0",
        1_000_215_216,
        &[("device/model", "SAMSUNG MZVL2512HDJD-00BH1       \n")],
    )?;
    fake.partition(NVME_PATH, "nvme0n1p1", 1, "259:1", 2_097_152)?;
    fake.partition(NVME_PATH, "nvme0n1p2", 2, "259:2", 997_000_000)?;
    fake.udev(
        "259:0",
        "S:disk/by-id/nvme-eui.002538bc31bf297e\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4_1\nE:ID_SERIAL_SHORT=S6Z4\nE:ID_MODEL=SAMSUNG MZVL2512HDJD-00BH1\n",
    )?;
    fake.udev(
        "259:1",
        "E:ID_FS_TYPE=vfat\nE:ID_PART_ENTRY_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b\n",
    )?;
    fake.udev("259:2", "E:ID_FS_TYPE=ext4\n")?;
    fake.mountinfo(
        "44 1 259:2 / / rw,noatime shared:1 - ext4 /dev/nvme0n1p2 rw,commit=60\n\
         64 44 259:1 / /boot/efi rw,relatime shared:134 - vfat /dev/nvme0n1p1 rw,fmask=0022\n\
         34 44 0:29 / /run rw,nosuid,nodev shared:13 - tmpfs tmpfs rw,size=3017756k\n",
    )
}

fn usb_stick(fake: &Fake) -> Result<(), Box<dyn Error>> {
    fake.disk(
        "sdb",
        USB_PATH,
        "8:16",
        31_260_672,
        &[
            ("removable", "1\n"),
            ("device/vendor", "SanDisk \n"),
            ("device/model", "Cruzer Blade    \n"),
        ],
    )?;
    fake.partition(USB_PATH, "sdb1", 1, "8:17", 31_258_624)?;
    fake.udev(
        "8:16",
        "S:disk/by-id/usb-SanDisk_Cruzer_Blade_4C530001-0:0\nE:ID_SERIAL_SHORT=4C530001\nE:ID_BUS=usb\n",
    )?;
    fake.udev("8:17", "E:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=CLE\n")
}

fn verdict(disk: &Disk) -> Eligibility {
    evaluate(disk, FilterOptions::default())
}

fn find<'a>(disks: &'a [Disk], path: &str) -> Result<&'a Disk, Box<dyn Error>> {
    disks
        .iter()
        .find(|d| d.path == path)
        .ok_or_else(|| format!("disque {path} absent de {disks:?}").into())
}

#[test]
fn disque_systeme_nvme_est_exclu() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    let disks = enumerate(&fake.roots())?;
    assert_eq!(disks.len(), 1);
    let nvme = &disks[0];
    assert_eq!(nvme.bus, Bus::Nvme);
    assert_eq!(nvme.kind, DiskKind::Physical);
    assert_eq!(nvme.size_bytes, 1_000_215_216 * 512);
    assert_eq!(nvme.model, "SAMSUNG MZVL2512HDJD-00BH1");
    assert_eq!(nvme.serial.as_deref(), Some("S6Z4"));
    // Le lien lisible l'emporte sur l'identifiant EUI ; le plus court sur sa variante « _1 ».
    assert_eq!(nvme.id, "nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4");
    let [esp, root] = nvme.partitions.as_slice() else {
        return Err(format!("deux partitions attendues : {:?}", nvme.partitions).into());
    };
    assert!(esp.is_active_esp && !esp.is_system_volume);
    assert_eq!(esp.mount_points, ["/boot/efi"]);
    assert!(root.is_system_volume && !root.is_active_esp);
    let Eligibility::Excluded { reasons } = verdict(nvme) else {
        return Err("le disque système doit être exclu".into());
    };
    assert!(reasons.contains(&Exclusion::SystemVolume {
        partition: "/dev/nvme0n1p2".to_owned()
    }));
    assert!(reasons.contains(&Exclusion::ActiveEsp {
        partition: "/dev/nvme0n1p1".to_owned()
    }));
    Ok(())
}

#[test]
fn cle_usb_montee_par_l_utilisateur_est_proposee() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
    info.push_str("90 44 8:17 / /run/media/alice/CLE rw,nosuid shared:2 - vfat /dev/sdb1 rw\n");
    fake.mountinfo(&info)?;
    let disks = enumerate(&fake.roots())?;
    let usb = find(&disks, "/dev/sdb")?;
    assert_eq!(usb.bus, Bus::Usb);
    assert!(usb.removable_flag);
    assert_eq!(usb.model, "SanDisk Cruzer Blade");
    assert_eq!(usb.id, "usb-SanDisk_Cruzer_Blade_4C530001-0:0");
    assert_eq!(usb.partitions[0].label.as_deref(), Some("CLE"));
    assert_eq!(usb.partitions[0].mount_points, ["/run/media/alice/CLE"]);
    assert!(verdict(usb).is_writable());
    Ok(())
}

#[test]
fn cle_usb_non_montee_est_proposee() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    let disks = enumerate(&fake.roots())?;
    assert!(verdict(find(&disks, "/dev/sdb")?).is_writable());
    Ok(())
}

#[test]
fn systeme_de_fichiers_sur_le_disque_entier_reste_visible() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fake.disk(
        "sdc",
        &USB_PATH.replace("sdb", "sdc"),
        "8:32",
        2_048_000,
        &[],
    )?;
    fake.udev("8:32", "E:ID_FS_TYPE=ext4\n")?;
    let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
    info.push_str("91 44 8:32 / /mnt/cle rw - ext4 /dev/sdc rw\n");
    fake.mountinfo(&info)?;
    let disks = enumerate(&fake.roots())?;
    let sdc = find(&disks, "/dev/sdc")?;
    assert!(sdc.partitions.len() == 1 && sdc.partitions[0].path == "/dev/sdc");
    assert_eq!(sdc.partitions[0].mount_points, ["/mnt/cle"]);
    assert!(!verdict(sdc).is_writable());
    Ok(())
}

#[test]
fn montage_btrfs_retrouve_par_la_source() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    // btrfs rapporte un numéro anonyme (0:50) : seule la source désigne le périphérique, ici
    // par un lien comme en crée udev.
    fs::create_dir_all(fake.base.join("dev/disk/by-uuid"))?;
    fs::write(fake.base.join("dev/sdb1"), "")?;
    symlink("../../sdb1", fake.base.join("dev/disk/by-uuid/abcd"))?;
    let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
    info.push_str("92 44 0:50 /sous-volume /mnt/donnees rw - btrfs /dev/disk/by-uuid/abcd rw\n");
    fake.mountinfo(&info)?;
    let disks = enumerate(&fake.roots())?;
    let usb = find(&disks, "/dev/sdb")?;
    assert_eq!(usb.partitions[0].mount_points, ["/mnt/donnees"]);
    assert!(!verdict(usb).is_writable());
    Ok(())
}

#[test]
fn point_de_montage_avec_espace() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
    info.push_str("90 44 8:17 / /run/media/alice/MA\\040CLE rw - vfat /dev/sdb1 rw\n");
    fake.mountinfo(&info)?;
    let disks = enumerate(&fake.roots())?;
    assert_eq!(
        find(&disks, "/dev/sdb")?.partitions[0].mount_points,
        ["/run/media/alice/MA CLE"]
    );
    Ok(())
}

#[test]
fn volume_chiffre_ouvert_est_empile() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    fake.udev("8:17", "E:ID_FS_TYPE=crypto_LUKS\n")?;
    fs::write(
        fake.base
            .join("sys")
            .join(USB_PATH)
            .join("sdb1/holders/dm-0"),
        "",
    )?;
    let disks = enumerate(&fake.roots())?;
    let usb = find(&disks, "/dev/sdb")?;
    assert_eq!(usb.partitions[0].holders, ["dm-0"]);
    assert!(!verdict(usb).is_writable());
    Ok(())
}

#[test]
fn swap_actif_detecte_meme_sans_signature_udev() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    fake.udev("8:17", "")?;
    fs::write(fake.base.join("dev/sdb1"), "")?;
    fake.put(
        "proc/swaps",
        "Filename\tType\tSize\tUsed\tPriority\n/dev/sdb1\tpartition\t1024\t0\t-2\n",
    )?;
    let disks = enumerate(&fake.roots())?;
    let usb = find(&disks, "/dev/sdb")?;
    assert_eq!(usb.partitions[0].fs_type.as_deref(), Some("swap"));
    assert!(!verdict(usb).is_writable());
    Ok(())
}

#[test]
fn disque_sata_et_carte_sd() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fake.disk("sda", SATA_PATH, "8:0", 1_953_525_168, &[])?;
    let sd = "devices/pci0000:00/0000:00:1a.0/mmc_host/mmc0/mmc0:0001/block/mmcblk0";
    fake.disk(
        "mmcblk0",
        sd,
        "179:0",
        31_116_288,
        &[("device/type", "SD\n")],
    )?;
    let emmc = "devices/platform/mmc_host/mmc1/mmc1:0001/block/mmcblk1";
    fake.disk(
        "mmcblk1",
        emmc,
        "179:8",
        15_269_888,
        &[("device/type", "MMC\n")],
    )?;
    let disks = enumerate(&fake.roots())?;
    assert_eq!(find(&disks, "/dev/sda")?.bus, Bus::Sata);
    assert_eq!(find(&disks, "/dev/mmcblk0")?.bus, Bus::Sd);
    assert_eq!(find(&disks, "/dev/mmcblk1")?.bus, Bus::Unknown);
    assert!(verdict(find(&disks, "/dev/mmcblk0")?).is_writable());
    assert!(!verdict(find(&disks, "/dev/mmcblk1")?).is_writable());
    Ok(())
}

#[test]
fn disques_virtuels_et_empiles() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    for (name, devnum) in [("loop0", "7:0"), ("zram0", "251:0"), ("nbd0", "43:0")] {
        fake.disk(
            name,
            &format!("devices/virtual/block/{name}"),
            devnum,
            2048,
            &[],
        )?;
    }
    for (name, devnum) in [
        ("dm-0", "253:0"),
        ("md127", "9:127"),
        ("sr0", "11:0"),
        ("ram0", "1:0"),
    ] {
        fake.disk(
            name,
            &format!("devices/virtual/block/{name}"),
            devnum,
            2048,
            &[],
        )?;
    }
    let disks = enumerate(&fake.roots())?;
    let names: Vec<&str> = disks.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        names,
        ["/dev/loop0", "/dev/nbd0", "/dev/nvme0n1", "/dev/zram0"]
    );
    assert_eq!(find(&disks, "/dev/loop0")?.kind, DiskKind::Loop);
    assert_eq!(find(&disks, "/dev/zram0")?.kind, DiskKind::Zram);
    assert_eq!(find(&disks, "/dev/nbd0")?.kind, DiskKind::HostVirtual);
    assert_eq!(find(&disks, "/dev/loop0")?.bus, Bus::Virtual);
    assert_eq!(find(&disks, "/dev/loop0")?.id, "dev-loop0");
    Ok(())
}

#[test]
fn nvme_cache_est_ignore() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fake.disk(
        "nvme0c0n1",
        "devices/pci0000:00/nvme/nvme0/nvme0c0n1",
        "259:9",
        2048,
        &[("hidden", "1\n")],
    )?;
    assert_eq!(enumerate(&fake.roots())?.len(), 1);
    Ok(())
}

#[test]
fn tri_naturel_des_disques() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    for (i, name) in ["loop10", "loop2", "loop1"].iter().enumerate() {
        fake.disk(
            name,
            &format!("devices/virtual/block/{name}"),
            &format!("7:{i}"),
            8,
            &[],
        )?;
    }
    let disks = enumerate(&fake.roots())?;
    let names: Vec<&str> = disks.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        names,
        ["/dev/loop1", "/dev/loop2", "/dev/loop10", "/dev/nvme0n1"]
    );
    Ok(())
}

#[test]
fn base_udev_absente_fait_echouer() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fs::remove_dir_all(fake.base.join("run/udev/data"))?;
    assert!(matches!(
        enumerate(&fake.roots()),
        Err(EnumerateError::Malformed { .. })
    ));
    Ok(())
}

#[test]
fn montages_illisibles_font_echouer() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fs::remove_file(fake.base.join("proc/self/mountinfo"))?;
    assert!(matches!(
        enumerate(&fake.roots()),
        Err(EnumerateError::Io { .. })
    ));
    fake.mountinfo("ligne sans separateur\n")?;
    assert!(matches!(
        enumerate(&fake.roots()),
        Err(EnumerateError::Malformed { .. })
    ));
    Ok(())
}

#[test]
fn taille_illisible_fait_echouer() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fake.put(&format!("sys/{NVME_PATH}/size"), "beaucoup\n")?;
    assert!(matches!(
        enumerate(&fake.roots()),
        Err(EnumerateError::Malformed { .. })
    ));
    Ok(())
}

#[test]
fn les_lectures_de_sources_sont_enregistrees_avec_leur_contenu() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    let (disks, reads) = enumerate_recorded(&fake.roots());
    assert_eq!(disks?.len(), 1);
    let mountinfo = reads
        .iter()
        .find(|r| r.path.ends_with("proc/self/mountinfo"))
        .ok_or("mountinfo non enregistré")?;
    assert!(mountinfo.ok && mountinfo.kind == "file");
    assert!(
        mountinfo
            .content
            .as_deref()
            .is_some_and(|c| c.contains("/boot/efi"))
    );
    // Les échecs sont enregistrés aussi : un fichier optionnel absent est une lecture ratée.
    assert!(
        reads.iter().any(|r| !r.ok && r.error.is_some()),
        "{reads:?}"
    );
    assert!(
        reads
            .iter()
            .any(|r| r.kind == "dir" && r.path.ends_with("sys/block"))
    );
    // L'enregistrement s'arrête avec l'appel : une énumération ordinaire ne garde rien.
    let again = enumerate(&fake.roots())?;
    assert_eq!(again.len(), 1);
    let (_, none_left) = enumerate_recorded(&fake.roots());
    assert_eq!(
        none_left
            .iter()
            .filter(|r| r.path.ends_with("proc/swaps"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn une_capture_rejouee_redonne_les_memes_disques() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    usb_stick(&fake)?;
    fake.disk("loop3", "devices/virtual/block/loop3", "7:3", 2048, &[])?;
    fake.partition(NVME_PATH, "nvme0n1p3", 3, "259:3", 2048)?;
    fs::write(
        fake.base
            .join("sys")
            .join(USB_PATH)
            .join("sdb1/holders/dm-0"),
        "",
    )?;
    let (original, reads) = enumerate_recorded(&fake.roots());
    let original = original?;
    assert!(
        reads
            .iter()
            .any(|r| r.kind == "link" && r.path.ends_with("sys/block/sdb")),
        "liens non enregistrés"
    );
    // Le fil de rejeu part d'un dossier vide : seul ce que la capture contient compte.
    let replayed = enumerate_from_reads(&reads)?;
    assert_eq!(replayed, original);
    // Le bus vient du chemin canonique : s'il manquait, la clé ne serait plus reconnue comme USB.
    assert_eq!(find(&replayed, "/dev/sdb")?.bus, Bus::Usb);
    assert_eq!(find(&replayed, "/dev/sdb")?.partitions[0].holders, ["dm-0"]);
    // Une capture sans les liens perd le bus : preuve que l'information passe bien par eux.
    let without_links: Vec<_> = reads.iter().filter(|r| r.kind != "link").cloned().collect();
    assert!(
        enumerate_from_reads(&without_links).map_or(true, |d| d.is_empty()
            || d.iter().all(|x| x.bus != Bus::Usb))
    );
    // Le dossier temporaire est supprimé.
    assert!(
        !std::env::temp_dir()
            .read_dir()?
            .filter_map(Result::ok)
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("bonk-fixture-{}-", std::process::id())))
    );
    Ok(())
}

#[test]
fn echappements_octaux() {
    assert_eq!(unescape_octal("/a\\040b"), "/a b");
    assert_eq!(unescape_octal("/a\\134b"), "/a\\b");
    assert_eq!(unescape_octal("/a\\04"), "/a\\04");
    assert_eq!(unescape_octal("/a\\"), "/a\\");
    assert_eq!(unescape_octal("/a\\999"), "/a\\999");
    assert_eq!(unescape_octal("/sans/echappement"), "/sans/echappement");
}

#[test]
fn ordre_naturel() {
    let mut names = vec!["loop10", "loop2", "sda", "nvme0n1", "loop1", "sdb", "sdaa"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        names,
        ["loop1", "loop2", "loop10", "nvme0n1", "sda", "sdaa", "sdb"]
    );
}
