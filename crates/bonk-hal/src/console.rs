use std::process::Command;

/// Leaves the console when this process is its only occupant: nobody reads it. In a terminal the shell also
/// occupies it, and it stays.
#[must_use]
pub fn detach_if_alone() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut occupants = [0u32; 2];
        // SAFETY: buffer and length come from the same array; neither call takes anything else.
        unsafe { GetConsoleProcessList(occupants.as_mut_ptr(), 2) == 1 && FreeConsole() != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Keeps a child from opening its own console when the window starts it.
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

    /// Tests run in a shared console or none: never alone, they keep it.
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
