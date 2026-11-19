/// Standard output of a `bonk` command, or its last error line.
pub(crate) fn run_json<S: AsRef<std::ffi::OsStr>>(
    program: &std::path::Path,
    args: &[S],
) -> Result<Vec<u8>, String> {
    let mut command = std::process::Command::new(program);
    bonk_hal::console::hide_child_console(&mut command);
    let output = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{}: cannot start: {e}", program.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("bonk: error: "));
        return Err(line.map_or_else(
            || output.status.to_string(),
            |l| l["bonk: error: ".len()..].to_owned(),
        ));
    }
    Ok(output.stdout)
}
