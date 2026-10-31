//! Commande équivalente d'une vue de la fenêtre : la fenêtre n'ayant aucun pouvoir que la
//! ligne de commande n'ait, chaque vue se traduit en commandes exactes. Le binaire vérifie par un test
//! que la CLI accepte bien celles que la fenêtre affiche.

/// Réglages de la fenêtre qui changent ce qu'elle montre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewOptions {
    pub show_internal: bool,
    pub allow_virtual: bool,
    pub show_all: bool,
}

/// Ce que le panneau d'écriture a de saisi.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteView<'a> {
    pub image: &'a str,
    /// Empreinte en hexadécimal, ou chemin ou adresse d'un fichier d'empreintes ; vide : aucune.
    pub hash: &'a str,
    pub raw: bool,
}

impl<'a> WriteView<'a> {
    #[must_use]
    pub fn image(image: &'a str) -> Self {
        Self {
            image,
            ..Self::default()
        }
    }

    /// Options de `bonk write` pour ce qui est saisi, en plus de l'image.
    #[must_use]
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        let hash = self.hash.trim();
        if !hash.is_empty() {
            let is_sha256 = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
            flags.push(
                if is_sha256 {
                    "--sha256"
                } else {
                    "--sha256-file"
                }
                .to_owned(),
            );
            flags.push(hash.to_owned());
        }
        if self.raw {
            flags.push("--mode".to_owned());
            flags.push("raw".to_owned());
        }
        flags
    }
}

/// Les commandes qui donnent ce que montre la fenêtre : la liste, le détail du disque sélectionné et,
/// quand une image est saisie pour l'écrire, l'écriture elle-même.
#[must_use]
pub fn equivalent_commands(
    options: ViewOptions,
    selected: Option<&str>,
    write: Option<WriteView<'_>>,
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
        if let Some(write) = write.filter(|write| !write.image.trim().is_empty()) {
            let extra: String = write
                .flags()
                .iter()
                .map(|flag| format!(" {}", shell_quote(flag)))
                .collect();
            commands.push(format!(
                "bonk write {} {}{extra} --yes{flags}",
                shell_quote(id),
                shell_quote(write.image.trim())
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
                Some(WriteView::image("/home/u/Mes images/debian.iso"))
            ),
            [
                "bonk list --show-internal",
                "bonk info usb-a --show-internal",
                "bonk write usb-a '/home/u/Mes images/debian.iso' --yes --show-internal",
            ]
        );
        // Sans disque sélectionné, ou avec une saisie vide, pas de commande d'écriture.
        assert_eq!(
            equivalent_commands(
                ViewOptions::default(),
                None,
                Some(WriteView::image("x.img"))
            )
            .len(),
            1
        );
        assert_eq!(
            equivalent_commands(
                ViewOptions::default(),
                Some("usb-a"),
                Some(WriteView::image("  "))
            )
            .len(),
            2
        );
    }

    #[test]
    fn empreinte_et_mode_passent_dans_la_commande() {
        let hex = "a".repeat(64);
        let view = WriteView {
            image: "https://a.org/x.iso",
            hash: &hex,
            raw: true,
        };
        let commands = equivalent_commands(ViewOptions::default(), Some("usb-a"), Some(view));
        assert_eq!(
            commands[2],
            format!("bonk write usb-a https://a.org/x.iso --sha256 {hex} --mode raw --yes")
        );
        let view = WriteView {
            image: "x.iso",
            hash: "https://a.org/SHA256SUMS",
            raw: false,
        };
        assert_eq!(view.flags(), ["--sha256-file", "https://a.org/SHA256SUMS"]);
    }
}
