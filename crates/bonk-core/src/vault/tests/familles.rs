use super::*;

#[test]
fn familles_reconnues_comme_par_le_menu() -> std::io::Result<()> {
    use crate::iso9660::{Iso, testing::image};
    use std::io::Cursor;
    let families = default_families();
    let menu = menu(&families);
    let family = |label: &str, files: &[(&str, u32)]| {
        iso_kind(&mut Iso::open(Cursor::new(image(label, files)))?, &families)
            .map(|kind| kind.map(|kind| kind.family.id))
    };
    for expected in &families {
        for set in &expected.detect {
            for file in set {
                assert!(menu.contains(&format!("(probe)/{file}")), "{file}");
            }
            let present: Vec<(&str, u32)> = set.iter().map(|f| (f.as_str(), 1)).collect();
            let label = format!(
                "{}Test",
                expected.label_prefix.as_deref().unwrap_or("Live-")
            );
            assert_eq!(
                family(&label, &present)?,
                Some(expected.id.clone()),
                "{label}"
            );
            assert_eq!(family("X", &present[..1])?, None, "{}", expected.id);
        }
    }
    let kiwi = [
        ("boot/x86_64/loader/linux", 1),
        ("boot/x86_64/loader/initrd", 1),
    ];
    assert_eq!(family("openSUSE-Live", &kiwi)?, None);
    let both = [
        ("live/vmlinuz", 1),
        ("live/initrd.img", 1),
        ("casper/vmlinuz", 1),
        ("casper/initrd", 1),
    ];
    assert_eq!(family("X", &both)?, Some("debian_live".to_owned()));
    let q4os = [
        ("live/vmlinuz-6.12.118+deb13-amd64", 1),
        ("live/initrd.img-6.12.118+deb13-amd64", 1),
    ];
    assert_eq!(
        family("Q4OS 6 Andromeda", &q4os)?,
        Some("debian_live_derivative".to_owned())
    );
    let two_kernels = [
        ("live/vmlinuz-6.12.118+deb13-amd64", 1),
        ("live/vmlinuz-6.12.119+deb13-amd64", 1),
        ("live/initrd.img-6.12.118+deb13-amd64", 1),
    ];
    assert_eq!(family("Q4OS 6 Andromeda", &two_kernels)?, None);
    Ok(())
}

#[test]
fn processeur_reconnu_comme_par_le_menu() -> std::io::Result<()> {
    use crate::iso9660::{Iso, testing::image};
    use std::io::Cursor;
    let arch = |label: &str, files: &[(&str, u32)]| {
        iso_kind(
            &mut Iso::open(Cursor::new(image(label, files)))?,
            &default_families(),
        )
        .map(|kind| kind.map(|kind| kind.arch))
    };
    let menu = menu(&default_families());
    for (file, _) in ARCH_RULES {
        assert!(menu.contains(&format!("(probe)/{file}")), "{file}");
    }
    let casper = [("casper/vmlinuz", 1), ("casper/initrd", 1)];
    assert_eq!(arch("Ubuntu", &casper)?, Some(IsoArch::X86_64));
    let arm = [
        ("casper/vmlinuz", 1),
        ("casper/initrd", 1),
        ("EFI/boot/bootaa64.efi", 1),
    ];
    assert_eq!(arch("Ubuntu", &arm)?, Some(IsoArch::Aarch64));
    let anaconda = [
        ("images/pxeboot/vmlinuz", 1),
        ("images/pxeboot/initrd.img", 1),
        ("images/install.img", 1),
        ("EFI/BOOT/BOOTAA64.EFI", 1),
    ];
    assert_eq!(
        arch("AlmaLinux-10-3-aarch64-dvd", &anaconda)?,
        Some(IsoArch::Aarch64)
    );
    assert_eq!(
        arch("AlmaLinux-10-3-x86_64-dvd", &anaconda[..3])?,
        Some(IsoArch::X86_64)
    );
    // Fedora's ARM64 image also carries an x86_64 loader: its kernel folder decides.
    let fedora = [
        ("boot/aarch64/loader/linux", 1),
        ("boot/aarch64/loader/initrd", 1),
        ("EFI/BOOT/bootx64.efi", 1),
    ];
    assert_eq!(arch("Fedora-WS-Live-44", &fedora)?, Some(IsoArch::Aarch64));
    let fedora = [
        ("boot/x86_64/loader/linux", 1),
        ("boot/x86_64/loader/initrd", 1),
        ("EFI/BOOT/BOOTAA64.EFI", 1),
    ];
    assert_eq!(arch("Fedora-WS-Live-44", &fedora)?, Some(IsoArch::X86_64));
    Ok(())
}

#[test]
fn les_familles_par_defaut_sont_valides_et_uniques() {
    let families = default_families();
    assert_eq!(families.len(), 16);
    for family in &families {
        assert_eq!(family_errors(family), Vec::<String>::new());
    }
    let mut ids: Vec<&str> = families.iter().map(|family| family.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), families.len());
}

#[test]
fn une_famille_qui_pourrait_sortir_du_menu_est_refusee() {
    let good = default_families().remove(0);
    type Spoil = fn(&mut VaultFamily);
    let cases: [(&str, Spoil); 12] = [
        ("identifier", |f| f.id = "Debian".to_owned()),
        ("name", |f| f.name = "Debian\" ; reboot".to_owned()),
        ("detect", |f| {
            f.detect = vec![vec!["../live/vmlinuz".to_owned()]]
        }),
        ("detect", |f| f.detect = vec![]),
        ("label prefix", |f| {
            f.label_prefix = Some("Fedora.*".to_owned())
        }),
        ("kernel", |f| f.kernel = "/live/vmlinuz".to_owned()),
        ("kernel", |f| f.kernel = "live/vmlinuz-*-*".to_owned()),
        ("kernel", |f| f.kernel = "live*/vmlinuz".to_owned()),
        ("detect", |f| f.detect = vec![vec!["*/vmlinuz".to_owned()]]),
        ("options", |f| f.options = "boot=live $root".to_owned()),
        ("options", |f| f.options = "findiso={iso}".to_owned()),
        ("initrd extra", |f| f.initrd_extra = Some("a b".to_owned())),
    ];
    for (reason, spoil) in cases {
        let mut family = good.clone();
        spoil(&mut family);
        let errors = family_errors(&family);
        assert!(
            errors.iter().any(|error| error.contains(reason)),
            "{reason}: {errors:?}"
        );
    }
}

#[test]
fn une_famille_servie_en_donnees_entre_au_menu_sans_code() -> Result<(), Box<dyn std::error::Error>>
{
    let opensuse: VaultFamily = serde_json::from_str(
        r#"{"id":"opensuse_live","name":"openSUSE live","detect":[["boot/x86_64/loader/linux","boot/x86_64/loader/initrd"]],
            "label_prefix":"openSUSE","kernel":"boot/{cpu}/loader/linux","initrd":"boot/{cpu}/loader/initrd",
            "options":"iso-scan/filename={path} root=live:CDLABEL={label} rd.live.image"}"#,
    )?;
    assert_eq!(family_errors(&opensuse), Vec::<String>::new());
    let mut families = default_families();
    families.push(opensuse);
    let menu = menu(&families);
    assert!(menu.contains(
        "if [ -z \"$family\" -a -f (probe)/boot/x86_64/loader/linux -a -f (probe)/boot/x86_64/loader/initrd ]; then\n    if regexp '^openSUSE' \"$isolabel\"; then set family=opensuse_live; fi\n  fi"
    ));
    assert!(menu.contains("menuentry \"openSUSE live: $path\""));
    assert!(menu.contains(
        "linux (loop)/boot/$5/loader/linux iso-scan/filename=$2 root=live:CDLABEL=$4 rd.live.image"
    ));
    Ok(())
}

#[test]
fn une_famille_servie_illisible_ou_refusee_est_laissee_de_cote() {
    let mut debian = serde_json::to_value(&default_families()[0]).unwrap_or_default();
    let mut unsafe_family = debian.clone();
    unsafe_family["options"] = serde_json::json!("boot=live; reboot");
    debian["id"] = serde_json::json!("debian_bis");
    let served = [serde_json::json!({"id": 3}), unsafe_family, debian];
    let kept: Vec<String> = families_from(&served).into_iter().map(|f| f.id).collect();
    assert_eq!(kept, ["debian_bis"]);
    assert_eq!(families_from(&served[..2]), default_families());
}
