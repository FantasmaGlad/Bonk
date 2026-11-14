#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use bonk_core::vault::CASPER_EXFAT;

type Res = Result<(), Box<dyn std::error::Error>>;

fn executable(path: &Path, body: &str) -> std::io::Result<()> {
    fs::write(path, body)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// The initrd step with its fixed paths moved under `root`, and a `blkid` that finds the partition at its
/// `found_at`-th call; `sleep` only counts, so the test never waits.
fn run_step(
    root: &Path,
    found_at: u32,
) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let at = root.display().to_string();
    for folder in ["scripts", "sbin", "conf", "bin"] {
        fs::create_dir_all(root.join(folder))?;
    }
    fs::write(
        root.join("scripts/casper-functions"),
        "wait_for_devs() { :; }\n",
    )?;
    fs::write(
        root.join("scripts/casper-helpers"),
        "panic() { echo \"$@\"; exit 1; }\n",
    )?;
    fs::write(root.join("cmdline"), "quiet bonk.offset=4096\n")?;
    executable(
        &root.join("sbin/blkid"),
        &format!(
            "#!/bin/sh\necho call >> {at}/calls\n[ \"$(wc -l < {at}/calls)\" -ge {found_at} ] && echo /dev/sda1\n"
        ),
    )?;
    executable(
        &root.join("bin/sleep"),
        &format!("#!/bin/sh\necho sleep >> {at}/sleeps\n"),
    )?;
    let step = CASPER_EXFAT
        .replace("/scripts/", &format!("{at}/scripts/"))
        .replace("/sbin/blkid", &format!("{at}/sbin/blkid"))
        .replace("/proc/cmdline", &format!("{at}/cmdline"))
        .replace("/conf/param.conf", &format!("{at}/conf/param.conf"));
    fs::write(root.join("step.sh"), step)?;
    let path = format!("{at}/bin:{}", std::env::var("PATH").unwrap_or_default());
    Ok(Command::new("sh")
        .arg(root.join("step.sh"))
        .env("PATH", path)
        .output()?)
}

fn scratch(name: &str) -> std::io::Result<std::path::PathBuf> {
    let root =
        std::env::temp_dir().join(format!("bonk-vault-ubuntu-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    Ok(root)
}

#[test]
fn une_cle_lente_a_apparaitre_est_attendue() -> Res {
    let root = scratch("lente")?;
    let output = run_step(&root, 3)?;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("conf/param.conf"))?,
        "LIVEMEDIA=/dev/sda1\nLIVEMEDIA_OFFSET=4096\n"
    );
    assert_eq!(fs::read_to_string(root.join("sleeps"))?.lines().count(), 2);
    fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
fn sans_partition_l_attente_finit_par_un_refus() -> Res {
    let root = scratch("absente")?;
    let output = run_step(&root, u32::MAX)?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("No BonkVault partition found"));
    assert_eq!(fs::read_to_string(root.join("sleeps"))?.lines().count(), 60);
    assert!(!root.join("conf/param.conf").exists());
    fs::remove_dir_all(&root)?;
    Ok(())
}
