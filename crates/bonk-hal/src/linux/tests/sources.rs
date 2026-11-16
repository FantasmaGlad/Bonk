use super::*;

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
    // Failures are recorded too: a missing optional file is a failed read.
    assert!(
        reads.iter().any(|r| !r.ok && r.error.is_some()),
        "{reads:?}"
    );
    assert!(
        reads
            .iter()
            .any(|r| r.kind == "dir" && r.path.ends_with("sys/block"))
    );
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
    let replayed = enumerate_from_reads(&reads)?;
    assert_eq!(replayed, original);
    assert_eq!(find(&replayed, "/dev/sdb")?.bus, Bus::Usb);
    assert_eq!(find(&replayed, "/dev/sdb")?.partitions[0].holders, ["dm-0"]);
    // Without the links the bus is lost: the information goes through them.
    let without_links: Vec<_> = reads.iter().filter(|r| r.kind != "link").cloned().collect();
    assert!(
        enumerate_from_reads(&without_links).map_or(true, |d| d.is_empty()
            || d.iter().all(|x| x.bus != Bus::Usb))
    );
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

#[test]
fn une_capture_ne_peut_pas_ecrire_hors_de_son_arbre() -> Result<(), Box<dyn Error>> {
    let read = |path: &str, kind: &str, content: &str| crate::enumerate::SourceRead {
        path: path.to_owned(),
        kind: kind.to_owned(),
        ok: true,
        bytes: None,
        error: None,
        micros: 0,
        content: Some(content.to_owned()),
    };
    let fake = Fake::new()?;
    let base = fake.base.join("arbre");
    let outside = fake.base.join("dehors");
    for reads in [
        vec![read("/sys/../../dehors", "file", "x")],
        vec![read("/../dehors", "file", "x")],
        vec![read("/sys/block/sdz/holders", "dir", "../../../dehors")],
        vec![read("/sys/block/sdz", "link", "../../dehors")],
    ] {
        let refused = replay::build_tree(&base, &reads).err().map(|e| e.kind());
        assert_eq!(refused, Some(std::io::ErrorKind::InvalidData), "{reads:?}");
        assert!(!outside.exists(), "{reads:?}");
    }
    replay::build_tree(&base, &[read("/sys/block/sdz/size", "file", "8")])?;
    assert_eq!(fs::read_to_string(base.join("sys/block/sdz/size"))?, "8");
    Ok(())
}
