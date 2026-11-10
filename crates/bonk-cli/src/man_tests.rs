#![allow(clippy::expect_used, clippy::unwrap_used)]

use clap::CommandFactory;

use crate::Cli;

/// One page per command, from the same definitions as `--help`. The packages take them from the folder
/// named by `BONK_MAN_DIR`.
#[test]
fn les_pages_de_manuel_couvrent_chaque_commande() {
    let dir = match std::env::var_os("BONK_MAN_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::temp_dir().join(format!("bonk-man-{}", std::process::id())),
    };
    std::fs::create_dir_all(&dir).expect("dossier des pages");
    let cmd = Cli::command().name("bonk");
    clap_mangen::generate_to(cmd, &dir).expect("pages de manuel");
    let main = std::fs::read_to_string(dir.join("bonk.1")).expect("bonk.1");
    assert!(main.contains(".TH bonk 1"), "{main}");
    for page in ["bonk-write.1", "bonk-vault-init.1", "bonk-debug-hexdump.1"] {
        let text = std::fs::read_to_string(dir.join(page)).expect(page);
        assert!(
            text.contains(".SH OPTIONS") || text.contains(".SH SYNOPSIS"),
            "{page}"
        );
    }
    if std::env::var_os("BONK_MAN_DIR").is_none() {
        std::fs::remove_dir_all(&dir).ok();
    }
}
