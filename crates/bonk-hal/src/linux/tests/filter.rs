use super::*;

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
    // The readable link wins over the EUI id; the shortest over its `_1` variant.
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
    assert_eq!(usb.logical_block_size, Some(512));
    assert_eq!(
        usb.location.as_deref(),
        Some("pci-0000:00:14.0-usb-0:2:1.0-scsi-0:0:0:0")
    );
    assert!(verdict(usb).is_writable());
    Ok(())
}

#[test]
fn un_peripherique_de_zero_octet_est_cache() -> Result<(), Box<dyn Error>> {
    let fake = Fake::new()?;
    desktop(&fake)?;
    fake.disk("loop25", "devices/virtual/block/loop25", "7:25", 0, &[])?;
    let disks = enumerate(&fake.roots())?;
    let empty = find(&disks, "/dev/loop25")?;
    let options = FilterOptions {
        allow_virtual: true,
        ..FilterOptions::default()
    };
    assert_eq!(
        crate::safety::evaluate(empty, options),
        Eligibility::Hidden {
            reason: crate::safety::HiddenReason::NoMedium
        }
    );
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
    // btrfs reports an anonymous number (0:50): only the source names the device, here through a udev-style
    // link.
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
