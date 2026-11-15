use std::process::{Command, Stdio};

pub(super) fn prefers_dark() -> Option<bool> {
    let output = Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(output) => Some(style_is_dark(output.status.success(), &output.stdout)),
        Err(error) => {
            log::info!(target: "bonk::hal", error:% = error; "appearance not read");
            None
        }
    }
}

// The key exists only in dark mode: `defaults` fails on a light system.
fn style_is_dark(found: bool, stdout: &[u8]) -> bool {
    found && stdout.trim_ascii() == b"Dark"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seul_le_style_dark_est_sombre() {
        assert!(style_is_dark(true, b"Dark\n"));
        assert!(!style_is_dark(false, b""));
        assert!(!style_is_dark(true, b"Light\n"));
    }
}
