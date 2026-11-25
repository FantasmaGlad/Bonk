use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How the vault menu recognises a live system in an ISO and starts it; served by the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VaultFamily {
    /// Stable identifier, `debian_live`.
    pub id: String,
    pub name: String,
    /// Files of the ISO, from its root: the family is recognised when one of these sets is all there.
    pub detect: Vec<Vec<String>>,
    /// The volume label must start with it: other images share the same layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_prefix: Option<String>,
    /// Kernel and initrd in the ISO; `{cpu}` is `x86_64` or `aarch64`.
    pub kernel: String,
    pub initrd: String,
    /// Kernel options: `{path}` of the ISO on the vault, its `{label}`, the `{offset}` `bonk vault add` records,
    /// the `{vault}` label and `{cpu}`.
    pub options: String,
    /// The initramfs cannot read the vault: `bonk vault add` records where the ISO lies, and the menu passes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub location: bool,
    /// Added to the initrd line, `{bonk}` being the folder of the menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initrd_extra: Option<String>,
}

const PLACEHOLDERS: [&str; 6] = [
    "{path}", "{label}", "{offset}", "{vault}", "{bonk}", "{cpu}",
];

#[must_use]
pub fn default_families() -> Vec<VaultFamily> {
    serde_json::from_str(include_str!("families.json")).unwrap_or_default()
}

// Written into the GRUB menu and the kernel line: a value that could close a quote, name a variable or chain a
// command is refused, whoever serves it.
#[must_use]
pub fn family_errors(family: &VaultFamily) -> Vec<String> {
    let mut errors = Vec::new();
    let mut check = |ok: bool, what: &str| {
        if !ok {
            errors.push(format!("vault family {}: {what}", family.id));
        }
    };
    check(
        is_id(&family.id),
        "identifier must be lowercase letters, digits and _",
    );
    check(
        is_name(&family.name),
        "name has a character the menu cannot show",
    );
    check(
        !family.detect.is_empty()
            && family
                .detect
                .iter()
                .all(|set| !set.is_empty() && set.iter().all(|path| is_path(path, false))),
        "detect needs sets of plain paths",
    );
    check(
        family.label_prefix.as_deref().is_none_or(is_label_prefix),
        "label prefix must be letters, digits, - or _",
    );
    check(
        is_path(&family.kernel, true) && is_path(&family.initrd, true),
        "kernel and initrd need plain paths",
    );
    check(
        !family.options.is_empty() && family.options.split(' ').all(is_token),
        "options need single-spaced plain tokens",
    );
    check(
        family.initrd_extra.as_deref().is_none_or(is_token),
        "initrd extra must be one plain token",
    );
    errors
}

fn is_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_name(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" ,.()+_-".contains(&b))
}

fn is_label_prefix(prefix: &str) -> bool {
    (1..=32).contains(&prefix.len())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn without_placeholders(text: &str) -> String {
    PLACEHOLDERS
        .iter()
        .fold(text.to_owned(), |text, placeholder| {
            text.replace(placeholder, "x")
        })
}

fn is_path(path: &str, cpu: bool) -> bool {
    let plain = if cpu {
        path.replace("{cpu}", "x")
    } else {
        path.to_owned()
    };
    !plain.is_empty()
        && plain.split('/').all(|part| {
            !part.is_empty()
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

fn is_token(token: &str) -> bool {
    let plain = without_placeholders(token);
    !plain.is_empty()
        && plain.len() <= 200
        && plain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/=+,-".contains(&b))
}
