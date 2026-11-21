use super::*;

/// A stderr lock held while the window is open would block forever every other thread that logs (enumeration,
/// timer) under `bonk -v`.
#[test]
fn un_autre_fil_peut_journaliser_pendant_que_la_fenetre_est_ouverte() {
    // Elevated (Windows runner), the window refuses to open before reaching what is tested here.
    if bonk_hal::platform::current().is_privileged() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let code = run(["bonk", "-v", "gui"], |_| {
            std::thread::spawn(|| {
                log::warn!(target: "bonk::gui", "journal depuis un autre fil");
            })
            .join()
            .map_err(|_| "le fil a paniqué".to_owned())
        });
        let _ = tx.send(code == ExitCode::SUCCESS);
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)),
        Ok(true),
        "blocage : le journal d'un autre fil attend le verrou de stderr"
    );
}

#[test]
fn sans_commande_la_fenetre_s_ouvre() {
    let r = lancer_avec(&[], None, vec![], Ok(()));
    assert_eq!(r.code, 0);
    assert!(r.fenetre_ouverte && r.journal_installe);
    assert!(r.out.is_empty());
}

#[test]
fn verbeux_sans_commande_ouvre_la_fenetre_avec_le_journal() {
    for args in [&["-v"][..], &["-vv"], &["gui", "-v"]] {
        let r = lancer_avec(args, None, vec![], Ok(()));
        assert_eq!(r.code, 0, "{args:?}");
        assert!(r.fenetre_ouverte && r.journal_installe, "{args:?}");
    }
}

#[test]
fn l_echec_de_la_fenetre_renvoie_vers_la_ligne_de_commande() {
    let r = lancer_avec(&[], None, vec![], Err("aucun affichage".to_owned()));
    assert_eq!(r.code, EXIT_RUNTIME_ERROR);
    assert!(
        r.err
            .starts_with("bonk: error: could not open the graphical interface: aucun affichage"),
        "{}",
        r.err
    );
    assert!(r.err.contains("bonk --help"), "{}", r.err);
}

#[test]
fn les_options_de_journal_sont_acceptees_apres_la_commande() {
    let r = lancer_avec(
        &[
            "list",
            "-vv",
            "--log-format",
            "json",
            "--log-file",
            "/tmp/x.log",
        ],
        None,
        sample(),
        Ok(()),
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.journal_installe);
}

#[test]
fn un_reglage_de_journal_invalide_est_une_erreur_d_usage() {
    let r = lancer_avec(&["list"], Some("gui=bavard"), sample(), Ok(()));
    assert_eq!(r.code, EXIT_USAGE);
    assert!(r.err.contains("unknown log level"), "{}", r.err);
    assert!(!r.journal_installe && r.out.is_empty());
}

#[test]
fn un_journal_impossible_a_installer_est_une_erreur_d_execution() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = execute(
        ["bonk", "list"],
        &Env::default(),
        &mut out,
        &mut err,
        |_| Err("journal : ouverture impossible".to_owned()),
        || Ok(vec![]),
        |_| Ok(()),
    );
    assert_eq!(code, EXIT_RUNTIME_ERROR);
    assert!(out.is_empty());
}

/// Exit code, the image handed to the window (`None` if it did not open), and the error text.
fn fenetre_demandee(args: &[&str]) -> (u8, Option<Option<String>>, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let recue = RefCell::new(None);
    let code = execute(
        std::iter::once("bonk").chain(args.iter().copied()),
        &Env::default(),
        &mut out,
        &mut err,
        |_| Ok(()),
        || Ok(vec![]),
        |request| {
            *recue.borrow_mut() = Some(request.image);
            Ok(())
        },
    );
    (
        code,
        recue.into_inner(),
        String::from_utf8_lossy(&err).into_owned(),
    )
}

#[test]
fn la_fenetre_s_ouvre_sur_l_image_nommee_ou_sur_celle_d_un_lien() {
    let image = Some(Some("ubuntu-26.04-desktop-x86_64".to_owned()));
    assert_eq!(
        fenetre_demandee(&["gui", "--image", "ubuntu-26.04-desktop-x86_64"]).1,
        image
    );
    assert_eq!(
        fenetre_demandee(&[
            "gui",
            "--link",
            "bonk://install/ubuntu-26.04-desktop-x86_64"
        ])
        .1,
        image
    );
    assert_eq!(fenetre_demandee(&["gui"]).1, Some(None));
    assert_eq!(fenetre_demandee(&[]).1, Some(None));
}

#[test]
fn un_lien_qui_n_est_pas_une_image_du_catalogue_n_ouvre_rien() {
    for args in [
        &["gui", "--link", "https://example.org/x.iso"][..],
        &["gui", "--link", "bonk://install/ubuntu?disk=sda"],
        &["gui", "--link", "bonk://write/ubuntu"],
        &["gui", "--image", "https://example.org/x.iso"],
        &["gui", "--image", "../x"],
    ] {
        let (code, recue, err) = fenetre_demandee(args);
        assert_eq!(code, EXIT_USAGE, "{args:?} {err}");
        assert_eq!(recue, None, "{args:?}");
        assert!(err.contains("[E"), "{err}");
    }
    let (code, recue, _) = fenetre_demandee(&["gui", "--link", "--yes"]);
    assert_eq!((code, recue), (EXIT_USAGE, None));
    let (code, recue, _) = fenetre_demandee(&["gui", "--image", "a", "--link", "bonk://install/a"]);
    assert_eq!((code, recue), (EXIT_USAGE, None));
}
