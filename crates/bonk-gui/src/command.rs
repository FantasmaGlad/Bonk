//! Commande équivalente d'une vue de la fenêtre (CDC §9.1) : la fenêtre n'ayant aucun pouvoir que la
//! ligne de commande n'ait, chaque vue se traduit en commandes exactes. Le binaire vérifie par un test
//! que la CLI accepte bien celles que la fenêtre affiche.

/// Réglages de la fenêtre qui changent ce qu'elle montre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewOptions {
    pub show_internal: bool,
    pub allow_virtual: bool,
    pub show_all: bool,
}

/// Les commandes qui donnent ce que montre la fenêtre : la liste, le détail du disque sélectionné et,
/// quand une image est saisie pour l'écrire, l'écriture elle-même.
#[must_use]
pub fn equivalent_commands(
    options: ViewOptions,
    selected: Option<&str>,
    write_image: Option<&str>,
) -> Vec<String> {
    let mut flags = String::new();
    if options.show_internal {
        flags.push_str(" --show-internal");
    }
    if options.allow_virtual {
        flags.push_str(" --allow-virtual");
    }
    let mut list = format!("bonk list{flags}");
    if options.show_all {
        list.push_str(" --all");
    }
    let mut commands = vec![list];
    if let Some(id) = selected {
        commands.push(format!("bonk info {}{flags}", shell_quote(id)));
        if let Some(image) = write_image.filter(|image| !image.trim().is_empty()) {
            commands.push(format!(
                "bonk write {} {} --yes{flags}",
                shell_quote(id),
                shell_quote(image.trim())
            ));
        }
    }
    commands
}

/// Entoure d'apostrophes ce qui ne serait pas un mot de shell sûr (syntaxe POSIX ; sous PowerShell,
/// les apostrophes simples ont aussi cet effet).
fn shell_quote(text: &str) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/=+@,".contains(c));
    if safe {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vue_par_defaut() {
        assert_eq!(
            equivalent_commands(ViewOptions::default(), None, None),
            ["bonk list"]
        );
    }

    #[test]
    fn options_et_selection() {
        let options = ViewOptions {
            show_internal: true,
            allow_virtual: true,
            show_all: true,
        };
        assert_eq!(
            equivalent_commands(options, Some("usb-SanDisk_Cruzer_4C53-0:0"), None),
            [
                "bonk list --show-internal --allow-virtual --all",
                "bonk info usb-SanDisk_Cruzer_4C53-0:0 --show-internal --allow-virtual",
            ]
        );
    }

    #[test]
    fn identifiant_a_proteger() {
        assert_eq!(shell_quote("clé usb"), "'clé usb'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a;rm -rf"), "'a;rm -rf'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("nvme-eui.0025"), "nvme-eui.0025");
    }
}

#[cfg(test)]
mod tests_write {
    use super::*;

    #[test]
    fn l_ecriture_apparait_avec_une_image_et_une_selection() {
        let options = ViewOptions {
            show_internal: true,
            ..ViewOptions::default()
        };
        assert_eq!(
            equivalent_commands(
                options,
                Some("usb-a"),
                Some("/home/u/Mes images/debian.iso")
            ),
            [
                "bonk list --show-internal",
                "bonk info usb-a --show-internal",
                "bonk write usb-a '/home/u/Mes images/debian.iso' --yes --show-internal",
            ]
        );
        // Sans disque sélectionné, ou avec une saisie vide, pas de commande d'écriture.
        assert_eq!(
            equivalent_commands(ViewOptions::default(), None, Some("x.img")).len(),
            1
        );
        assert_eq!(
            equivalent_commands(ViewOptions::default(), Some("usb-a"), Some("  ")).len(),
            2
        );
    }
}
