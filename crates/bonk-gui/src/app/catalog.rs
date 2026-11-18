use std::path::PathBuf;

pub(crate) use bonk_ui::catalog::*;

use super::command::run_json;

/// In a separate process, through `proxy` when given. The last error line says why it failed.
pub(crate) fn read(program: PathBuf, proxy: Option<&str>) -> Result<Shelf, String> {
    let mut args = vec!["catalog", "list", "--json"];
    if let Some(proxy) = proxy {
        args.extend(["--proxy", proxy]);
    }
    let output = run_json(&program, &args)?;
    serde_json::from_slice(&output)
        .map(|report| bonk_ui::catalog::shelf(&report))
        .map_err(|e| e.to_string())
}
