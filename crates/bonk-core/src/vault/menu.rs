use std::fmt::Write;

use super::{ARCH_RULES, IsoArch, VAULT_LABEL, VaultFamily};

const HEAD: &str = include_str!("menu/head.cfg");
const TAIL: &str = include_str!("menu/tail.cfg");

#[must_use]
pub fn menu(families: &[VaultFamily]) -> String {
    let mut menu = String::from(HEAD);
    menu.push_str(&detect(families));
    menu.push_str(&add_entry(families));
    menu.push('\n');
    menu.push_str(TAIL);
    menu
}

fn detect(families: &[VaultFamily]) -> String {
    let mut out = String::from(
        "# Sets `family` (empty when not supported) and `iso_cpu` for the ISO mounted as (probe).\nfunction detect {\n  unset family\n",
    );
    let mut keyword = "if";
    for family in families {
        for set in &family.detect {
            let tests: Vec<String> = set
                .iter()
                .map(|file| format!("-f (probe)/{file}"))
                .collect();
            let _ = write!(
                out,
                "  {keyword} [ {} ]; then\n    set family={}\n",
                tests.join(" -a "),
                family.id
            );
            keyword = "elif";
        }
    }
    if keyword == "elif" {
        out.push_str("  fi\n");
    }
    out.push_str("  unset isolabel\n  probe --set=isolabel --label (probe)\n");
    for family in families {
        if let Some(prefix) = &family.label_prefix {
            let _ = write!(
                out,
                "  if [ \"$family\" = {id} ]; then\n    if regexp '^{prefix}' \"$isolabel\"; then set family={id}; else unset family; fi\n  fi\n",
                id = family.id
            );
        }
    }
    for (index, (file, arch)) in ARCH_RULES.iter().enumerate() {
        let keyword = if index == 0 { "if" } else { "elif" };
        let _ = write!(
            out,
            "  {keyword} [ -f (probe)/{file} ]; then\n    set iso_cpu={}\n",
            grub_cpu(*arch)
        );
    }
    out.push_str("  else\n    set iso_cpu=x86_64\n  fi\n}\n");
    out
}

fn grub_cpu(arch: IsoArch) -> &'static str {
    match arch {
        IsoArch::X86_64 => "x86_64",
        IsoArch::Aarch64 => "arm64",
    }
}

fn add_entry(families: &[VaultFamily]) -> String {
    let mut out = String::from(
        "\n# The entry of an ISO this machine can start, or the reason it cannot.\nfunction add_entry {\n  set kernel_cpu=x86_64\n  if [ \"$iso_cpu\" = arm64 ]; then set kernel_cpu=aarch64; fi\n  # The kernel line is split on spaces: no live system would find this ISO again.\n  if regexp ' ' \"$path\"; then\n    menuentry \"Rename without spaces: $path\" {\n      echo \"Spaces in the ISO path cannot be passed to the system being started.\"\n      sleep 5\n    }\n",
    );
    for family in families {
        let _ = writeln!(out, "  elif [ \"$family\" = {} ]; then", family.id);
        out.push_str(&entry(family));
    }
    out.push_str(
        "  else\n    menuentry \"Not supported by this vault: $path\" {\n      echo \"This ISO is not in the vault's compatibility list.\"\n      sleep 5\n    }\n  fi\n}\n",
    );
    out
}

// Arguments of every entry: $2 the ISO path, $3 its location file, $4 its label, $5 the kernel's processor.
fn entry(family: &VaultFamily) -> String {
    let fill = |text: &str| {
        text.replace("{path}", "$2")
            .replace("{label}", "$4")
            .replace("{cpu}", "$5")
            .replace("{offset}", "$bonk_offset")
            .replace("{vault}", VAULT_LABEL)
            .replace("{bonk}", "$bonk_dir")
    };
    let mut lines = Vec::new();
    if family.location {
        lines.push("source \"$3\"".to_owned());
    }
    lines.push("loopback loop ($vault)$2".to_owned());
    lines.push(format!(
        "linux (loop)/{} {}",
        fill(&family.kernel),
        fill(&family.options)
    ));
    lines.push(match &family.initrd_extra {
        Some(extra) => format!("initrd (loop)/{} {}", fill(&family.initrd), fill(extra)),
        None => format!("initrd (loop)/{}", fill(&family.initrd)),
    });
    let title = format!(
        "\"{}: $path\" \"$path\" \"$1.bonk\" \"$isolabel\" \"$kernel_cpu\"",
        family.name
    );
    if !family.location {
        return menuentry("    ", &title, &lines);
    }
    let refusal = [
        format!(
            "echo \"{} starts from the vault only when added by bonk vault add, which records where it lies.\"",
            family.name
        ),
        "sleep 5".to_owned(),
    ];
    format!(
        "    if [ -f \"$1.bonk\" ]; then\n{}    else\n{}    fi\n",
        menuentry("      ", &title, &lines),
        menuentry(
            "      ",
            &format!("\"{}, add it with bonk vault add: $path\"", family.name),
            &refusal
        ),
    )
}

fn menuentry(indent: &str, title: &str, lines: &[String]) -> String {
    let body: String = lines
        .iter()
        .map(|line| format!("{indent}  {line}\n"))
        .collect();
    format!("{indent}menuentry {title} {{\n{body}{indent}}}\n")
}
