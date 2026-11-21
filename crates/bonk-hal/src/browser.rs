use std::process::{Command, Stdio};

pub fn open(url: &str) -> bool {
    let Some(mut command) = opener(url) else {
        return false;
    };
    crate::console::hide_child_console(&mut command);
    let started = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match started {
        Ok(mut child) => {
            // Reaped in the background: the opener exits at once, the window keeps running.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(error) => {
            log::info!(target: "bonk::hal", error:% = error; "browser not opened");
            false
        }
    }
}

fn opener(url: &str) -> Option<Command> {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("rundll32", &["url.dll,FileProtocolHandler"])
    } else if cfg!(target_os = "macos") {
        ("/usr/bin/open", &[])
    } else if cfg!(target_os = "linux") {
        ("xdg-open", &[])
    } else {
        return None;
    };
    let mut command = Command::new(program);
    command.args(args).arg(url);
    Some(command)
}
