use bonk_core::format::table::TableKind;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewOptions {
    pub show_internal: bool,
    pub allow_virtual: bool,
    pub show_all: bool,
    /// Window only, no command flag: no catalog read, no download.
    pub offline: bool,
    /// Window only: start in incognito, nothing on the network until Tor answers.
    pub incognito: bool,
    /// Window and phone app: each step of a write shown in the system's notifications.
    pub notify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteView<'a> {
    pub image: &'a str,
    /// Hex hash, or path or address of a checksum file; empty: none.
    pub hash: &'a str,
    pub raw: bool,
    /// MBR for a Windows drive, the `bonk write` default; GPT otherwise.
    pub mbr: bool,
    /// An internal disk, confirmed in the window: `bonk write` requires `--force` for it.
    pub force: bool,
}

impl Default for WriteView<'_> {
    fn default() -> Self {
        Self {
            image: "",
            hash: "",
            raw: false,
            mbr: TableKind::WINDOWS_DEFAULT == TableKind::Mbr,
            force: false,
        }
    }
}

impl<'a> WriteView<'a> {
    #[must_use]
    pub fn image(image: &'a str) -> Self {
        Self {
            image,
            ..Self::default()
        }
    }

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
        if !self.mbr {
            flags.push("--table".to_owned());
            flags.push("gpt".to_owned());
        }
        if self.force {
            flags.push("--force".to_owned());
        }
        flags
    }
}

#[must_use]
pub fn cache_list_args() -> Vec<String> {
    ["cache", "list", "--json"].map(str::to_owned).into()
}

#[must_use]
pub fn cache_clean_args(sha256: &str) -> Vec<String> {
    ["cache", "clean", sha256, "--json"]
        .map(str::to_owned)
        .into()
}

#[must_use]
pub fn config_set_args(key: &str, value: &str) -> Vec<String> {
    ["config", "set", key, value].map(str::to_owned).into()
}

/// Commands giving what the window shows: the list, the selected disk's detail and, when an image is entered,
/// the write itself.
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

/// Quotes what would not be a safe shell word (POSIX; in PowerShell single quotes do the same).
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
            offline: false,
            incognito: false,
            notify: false,
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
            mbr: true,
            force: false,
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
            mbr: false,
            force: false,
        };
        assert_eq!(
            view.flags(),
            [
                "--sha256-file",
                "https://a.org/SHA256SUMS",
                "--table",
                "gpt"
            ]
        );
    }

    #[test]
    fn un_disque_interne_confirme_porte_force_comme_la_commande_lancee() {
        let view = WriteView {
            force: true,
            ..WriteView::image("x.iso")
        };
        let commands = equivalent_commands(ViewOptions::default(), Some("ata-a"), Some(view));
        assert_eq!(commands[2], "bonk write ata-a x.iso --force --yes");
        assert!(
            crate::event::write_args("ata-a", view, ViewOptions::default(), None)
                .contains(&"--force".to_owned())
        );
    }
}
