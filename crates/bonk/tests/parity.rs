//! Parité fenêtre et ligne de commande : la fenêtre n'a aucun pouvoir que la CLI n'ait.
//! Chaque commande équivalente qu'elle affiche, pour toute combinaison de réglages et de sélection,
//! doit être acceptée par la CLI, et la cible d'`info` doit arriver intacte, même avec des caractères
//! que le shell interpréterait.

use bonk_gui::{ViewOptions, equivalent_commands};

/// Images saisies dans le champ de la fenêtre, y compris des chemins que le shell interpréterait.
const IMAGES: [&str; 4] = [
    "/tmp/debian.iso",
    "/home/u/Mes images/ubuntu 24.04.img.xz",
    "/tmp/d'artagnan; rm -rf ~.img",
    "~/téléchargements/$HOME.img",
];

#[test]
fn toutes_les_commandes_equivalentes_sont_acceptees_par_la_cli() -> Result<(), String> {
    let identifiants = [
        None,
        Some("usb-SanDisk_Cruzer_Blade_4C530001-0:0"),
        Some("nvme-eui.002538bc31bf297e"),
        Some("dev-loop0"),
        Some("clé usb avec espace"),
        Some("it's; rm -rf /"),
    ];
    let mut verifiees = 0;
    for (bits, image) in (0..8u8).flat_map(|b| IMAGES.iter().map(move |i| (b, *i))) {
        let options = ViewOptions {
            show_internal: bits & 1 != 0,
            allow_virtual: bits & 2 != 0,
            show_all: bits & 4 != 0,
        };
        for selection in identifiants {
            let commandes = equivalent_commands(options, selection, Some(image));
            assert_eq!(commandes.len(), 1 + 2 * usize::from(selection.is_some()));
            for commande in &commandes {
                let mots = bonk_cli::check_command_line(commande)
                    .map_err(|e| format!("{commande:?} refusée : {e}"))?;
                verifiees += 1;
                // Les réglages affichés sont exactement ceux de la vue.
                assert_eq!(
                    mots.iter().any(|m| m == "--all"),
                    options.show_all && mots[1] == "list"
                );
                assert_eq!(
                    mots.iter().any(|m| m == "--show-internal"),
                    options.show_internal
                );
                assert_eq!(
                    mots.iter().any(|m| m == "--allow-virtual"),
                    options.allow_virtual
                );
                if mots[1] == "info" || mots[1] == "write" {
                    assert_eq!(Some(mots[2].as_str()), selection, "{commande:?}");
                }
                if mots[1] == "write" {
                    // L'image arrive intacte, et la confirmation fait partie de la commande copiée.
                    assert_eq!(mots[3], image, "{commande:?}");
                    assert!(mots.iter().any(|m| m == "--yes"));
                }
            }
        }
    }
    assert_eq!(verifiees, 8 * IMAGES.len() * (6 + 5 * 2));
    Ok(())
}
