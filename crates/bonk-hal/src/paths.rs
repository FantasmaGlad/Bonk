use std::path::PathBuf;

#[must_use]
pub fn config_file(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    config_file_for(std::env::consts::OS, var)
}

#[must_use]
pub fn cache_dir(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    cache_dir_for(std::env::consts::OS, var)
}

fn cache_dir_for(os: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match os {
        "windows" => var("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Bonk").join("cache")),
        "macos" => var("HOME").map(|home| PathBuf::from(home).join("Library/Caches/Bonk")),
        _ => var("XDG_CACHE_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|dir| dir.join("bonk")),
    }
}

/// Takes the OS as input, so each system's rule is tested everywhere.
fn config_file_for(os: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match os {
        "windows" => var("APPDATA").map(|dir| PathBuf::from(dir).join("Bonk").join("config.json")),
        "macos" => var("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Bonk/config.json")),
        _ => var("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join("bonk").join("config.json")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn le_cache_suit_les_regles_de_chaque_systeme() {
        let path = |os, pairs| cache_dir_for(os, &env(pairs));
        assert_eq!(
            path("windows", &[("LOCALAPPDATA", "L")]),
            Some(PathBuf::from("L").join("Bonk").join("cache"))
        );
        assert_eq!(
            path("macos", &[("HOME", "H")]),
            Some(PathBuf::from("H").join("Library/Caches/Bonk"))
        );
        assert_eq!(
            path("linux", &[("XDG_CACHE_HOME", "X"), ("HOME", "H")]),
            Some(PathBuf::from("X").join("bonk"))
        );
        assert_eq!(
            path("linux", &[("XDG_CACHE_HOME", ""), ("HOME", "H")]),
            Some(PathBuf::from("H").join(".cache").join("bonk"))
        );
        assert_eq!(path("linux", &[]), None);
    }

    #[test]
    fn le_fichier_de_configuration_suit_les_regles_de_chaque_systeme() {
        let path = |os, pairs| config_file_for(os, &env(pairs));
        assert_eq!(
            path("windows", &[("APPDATA", "A")]),
            Some(PathBuf::from("A").join("Bonk").join("config.json"))
        );
        assert_eq!(
            path("macos", &[("HOME", "H")]),
            Some(PathBuf::from("H").join("Library/Application Support/Bonk/config.json"))
        );
        assert_eq!(
            path("linux", &[("XDG_CONFIG_HOME", "X"), ("HOME", "H")]),
            Some(PathBuf::from("X").join("bonk").join("config.json"))
        );
        // An empty variable counts as unset (XDG specification).
        assert_eq!(
            path("linux", &[("XDG_CONFIG_HOME", ""), ("HOME", "H")]),
            Some(
                PathBuf::from("H")
                    .join(".config")
                    .join("bonk")
                    .join("config.json")
            )
        );
        assert_eq!(path("linux", &[]), None);
    }
}
