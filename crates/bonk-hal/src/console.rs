//! Console de Windows : un seul binaire sert la ligne de commande et la fenêtre. Marqué console, il garde sa
//! sortie dans un terminal ; lancé d'un double-clic, il se défait de la console que Windows lui a ouverte.
//! Ailleurs, rien à faire.

use std::process::Command;

/// Quitte la console si ce processus en est le seul occupant : personne ne la lit. Rend vrai si c'est fait. Dans
/// un terminal, la console a aussi le shell comme occupant et reste.
#[must_use]
pub fn detach_if_alone() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut occupants = [0u32; 2];
        // SAFETY : le tampon et sa longueur viennent du même tableau ; les deux appels n'ont pas d'autre entrée.
        unsafe { GetConsoleProcessList(occupants.as_mut_ptr(), 2) == 1 && FreeConsole() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Empêche un sous-processus d'ouvrir sa propre console quand la fenêtre le lance.
pub fn hide_child_console(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use super::*;

    /// Les tests tournent dans une console partagée ou sans console : jamais seuls, ils la gardent.
    #[test]
    fn une_console_partagee_reste_en_place() {
        assert!(!detach_if_alone());
    }

    #[test]
    fn un_sous_processus_sans_console_lit_et_ecrit_par_ses_tuyaux() -> std::io::Result<()> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--list")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = hide_child_console(&mut command).output()?;
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        Ok(())
    }
}
